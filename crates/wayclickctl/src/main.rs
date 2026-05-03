// SPDX-License-Identifier: MIT
use clap::{Parser, Subcommand, ValueEnum};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wayclick_ipc_client::frame::{decode_frame, write_frame};
use wayclick_ipc_client::socket::default_socket_path;
use wayclick_ipc_client::{connect_with_timeout, SyncClient};

#[derive(Parser)]
#[command(name = "wayclickctl", about = "Wayclick daemon control CLI")]
#[command(version)]
struct Cli {
    /// IPC socket path
    #[arg(long)]
    socket: Option<String>,

    /// Output raw JSON response
    #[arg(long)]
    json: bool,

    /// Connection timeout in milliseconds
    #[arg(long, default_value = "2000")]
    timeout: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show daemon status
    Status,
    /// Toggle automation on/off
    Toggle,
    /// Enable automation
    Enable,
    /// Disable automation
    Disable,
    /// Fire or control a named trigger
    Trigger {
        #[command(subcommand)]
        action: TriggerAction,
    },
    /// List all triggers with current state
    List,
    /// Reload configuration
    Reload,
    /// Show recent log entries
    Logs {
        /// Number of entries to show
        #[arg(long, default_value = "50")]
        tail: usize,
    },
    /// Check if daemon is running
    Ping,
    /// Manage layers
    Layer {
        #[command(subcommand)]
        action: LayerAction,
    },
    /// Subscribe to daemon events and print them to stdout
    Watch {
        /// Output raw JSON instead of human-readable format
        #[arg(long)]
        json: bool,
    },
    /// Validate a Lua config file without applying it
    CheckConfig {
        /// Path to the Lua config file to validate
        path: PathBuf,
    },
    /// Output Waybar-compatible JSON for status display
    Waybar {
        /// Run continuously, updating on every daemon event (recommended)
        #[arg(long)]
        continuous: bool,

        /// Poll interval in seconds (polling mode only, ignored in continuous/event-driven mode)
        #[arg(long, default_value = "2")]
        interval: u64,

        /// Display format
        #[arg(long, default_value = "normal")]
        format: WaybarFormat,

        /// How long to hold the `triggering` CSS class after a trigger fires (ms)
        #[arg(long, default_value = "400")]
        flash_ms: u64,
    },
}

#[derive(Subcommand)]
enum TriggerAction {
    /// Fire a trigger
    Fire {
        /// Trigger ID to fire
        id: String,
    },
    /// Enable a previously-disabled trigger
    Enable {
        /// Trigger ID
        id: String,
    },
    /// Disable a trigger without removing it
    Disable {
        /// Trigger ID
        id: String,
    },
}

#[derive(Subcommand)]
enum LayerAction {
    /// Get the current active layer
    Get,
    /// Set the active layer
    Set {
        /// Layer name to switch to
        name: String,
    },
    /// List all available layers
    List,
    /// Cycle to the next (or previous) layer
    Cycle {
        /// Cycle backwards instead of forwards
        #[arg(long)]
        backward: bool,
    },
}

#[derive(Clone, ValueEnum)]
enum WaybarFormat {
    /// Icon only
    Minimal,
    /// Icon and layer name (default)
    Normal,
    /// Icon, layer name, and active trigger count
    Verbose,
    /// Icon followed by per-trigger state dots (● active, ○ idle)
    Triggers,
}

fn main() {
    let cli = Cli::parse();

    let socket_path = match &cli.socket {
        Some(p) => PathBuf::from(p),
        None => default_socket_path(),
    };

    let result = match &cli.command {
        Command::Ping => SyncClient::request(&socket_path, "ping", None),
        Command::Status => SyncClient::request(&socket_path, "status", None),
        Command::Toggle => SyncClient::request(&socket_path, "toggle", None),
        Command::Enable => SyncClient::request(&socket_path, "enable", None),
        Command::Disable => SyncClient::request(&socket_path, "disable", None),
        Command::Trigger { action } => match action {
            TriggerAction::Fire { id } => SyncClient::request(
                &socket_path,
                "trigger",
                Some(serde_json::json!({ "id": id, "press": true })),
            ),
            TriggerAction::Enable { id } => SyncClient::request(
                &socket_path,
                "enable_trigger",
                Some(serde_json::json!({ "id": id })),
            ),
            TriggerAction::Disable { id } => SyncClient::request(
                &socket_path,
                "disable_trigger",
                Some(serde_json::json!({ "id": id })),
            ),
        },
        Command::List => SyncClient::request(&socket_path, "list_triggers", None),
        Command::Reload => SyncClient::request(&socket_path, "reload_config", None),
        Command::Logs { tail } => SyncClient::request(
            &socket_path,
            "logs_tail",
            Some(serde_json::json!({ "n": tail })),
        ),
        Command::Layer { action } => match action {
            LayerAction::Get => SyncClient::request(&socket_path, "get_layer", None),
            LayerAction::Set { name } => SyncClient::request(
                &socket_path,
                "set_layer",
                Some(serde_json::json!({ "layer": name })),
            ),
            LayerAction::List => SyncClient::request(&socket_path, "list_layers", None),
            LayerAction::Cycle { backward } => {
                run_layer_cycle(&socket_path, *backward);
                return;
            }
        },
        Command::Watch { json } => {
            run_watch(&socket_path, *json);
            return;
        }
        Command::CheckConfig { path } => {
            let abs = match path.canonicalize() {
                Ok(p) => p,
                Err(_) => path.clone(),
            };
            SyncClient::request(
                &socket_path,
                "check_config",
                Some(serde_json::json!({ "path": abs.to_string_lossy() })),
            )
        }
        Command::Waybar {
            continuous,
            interval,
            format,
            flash_ms,
        } => {
            run_waybar(&socket_path, *continuous, *interval, format, *flash_ms);
            return;
        }
    };

    match result {
        Ok(response) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&response).unwrap());
                return;
            }

            if let Some(err) = response.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error");
                eprintln!("Error: {}", msg);
                std::process::exit(2);
            }

            let result = &response["result"];

            match &cli.command {
                Command::Ping => {
                    println!("{}", result.as_str().unwrap_or("ok"));
                }
                Command::Status => {
                    println!("Enabled:    {}", result["enabled"]);
                    println!("Dry run:    {}", result["dry_run"]);
                    println!("Triggers:   {}", result["trigger_count"]);
                    if let Some(active) = result["active_triggers"].as_array() {
                        if active.is_empty() {
                            println!("Active:     (none)");
                        } else {
                            let names: Vec<&str> =
                                active.iter().filter_map(|v| v.as_str()).collect();
                            println!("Active:     {}", names.join(", "));
                        }
                    }
                    println!("Backend:    {}", result["backend"].as_str().unwrap_or("?"));
                    println!(
                        "Config:     {}",
                        result["config_path"].as_str().unwrap_or("?")
                    );
                    println!("Layer:      {}", result["layer"].as_str().unwrap_or("base"));
                    println!("Uptime:     {}s", result["uptime_secs"]);
                }
                Command::Toggle => {
                    let enabled = result["enabled"].as_bool().unwrap_or(false);
                    println!("{}", if enabled { "enabled" } else { "disabled" });
                }
                Command::Enable => println!("enabled"),
                Command::Disable => println!("disabled"),
                Command::Trigger { action } => match action {
                    TriggerAction::Fire { id } => println!("Triggered: {}", id),
                    TriggerAction::Enable { id } => println!("Trigger enabled: {}", id),
                    TriggerAction::Disable { id } => println!("Trigger disabled: {}", id),
                },
                Command::List => {
                    if let Some(triggers) = result.as_array() {
                        if triggers.is_empty() {
                            println!("No triggers configured");
                        } else {
                            for t in triggers {
                                let id = t["id"].as_str().unwrap_or("?");
                                let mode = t["mode"].as_str().unwrap_or("?");
                                let active = t["active"].as_bool().unwrap_or(false);
                                let action = t["action_type"].as_str().unwrap_or("?");
                                let enabled = t["user_enabled"].as_bool().unwrap_or(true);
                                let count = t["activate_count"].as_u64().unwrap_or(0);
                                let status = if !enabled {
                                    "DISABLED"
                                } else if active {
                                    "ACTIVE"
                                } else {
                                    "idle"
                                };
                                println!(
                                    "  {} {:<22} [{:<8}] {:<10} {:<16} {:>5}×",
                                    if active {
                                        "●"
                                    } else if !enabled {
                                        "✗"
                                    } else {
                                        "○"
                                    },
                                    id,
                                    status,
                                    mode,
                                    action,
                                    count,
                                );
                            }
                        }
                    }
                }
                Command::Reload => {
                    println!("Reload requested");
                }
                Command::Logs { .. } => {
                    if let Some(logs) = result.as_array() {
                        for entry in logs {
                            let level = entry["level"].as_str().unwrap_or("?");
                            let msg = entry["message"].as_str().unwrap_or("");
                            let ts = entry["timestamp"].as_str().unwrap_or("");
                            println!("[{}] [{}] {}", ts, level, msg);
                        }
                    }
                }
                Command::Layer { action } => match action {
                    LayerAction::Get => {
                        println!("{}", result["layer"].as_str().unwrap_or("base"));
                    }
                    LayerAction::Set { name } => {
                        println!("Layer set to: {}", name);
                    }
                    LayerAction::List => {
                        let current = result["current"].as_str().unwrap_or("base");
                        if let Some(layers) = result["layers"].as_array() {
                            for layer in layers {
                                if let Some(name) = layer.as_str() {
                                    if name == current {
                                        println!("  * {}", name);
                                    } else {
                                        println!("    {}", name);
                                    }
                                }
                            }
                        }
                    }
                    LayerAction::Cycle { .. } => unreachable!(),
                },
                Command::Watch { .. } => unreachable!(),
                Command::CheckConfig { .. } => {
                    let valid = result["valid"].as_bool().unwrap_or(false);
                    if valid {
                        println!("Config is valid");
                    } else {
                        println!("Config has errors:");
                        if let Some(errors) = result["errors"].as_array() {
                            for err in errors {
                                if let Some(msg) = err.as_str() {
                                    println!("  - {}", msg);
                                }
                            }
                        }
                        std::process::exit(1);
                    }
                }
                Command::Waybar { .. } => unreachable!(),
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("No such file") || msg.contains("Connection refused") {
                eprintln!("Error: daemon is not running (socket: {:?})", socket_path);
            } else {
                eprintln!("Error: {}", msg);
            }
            std::process::exit(1);
        }
    }
}

// ── Layer cycle ────────────────────────────────────────────────────────────────

fn run_layer_cycle(socket_path: &std::path::Path, backward: bool) {
    let layers_resp = match SyncClient::request(socket_path, "list_layers", None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let result = &layers_resp["result"];
    let current = result["current"].as_str().unwrap_or("base").to_string();
    let layers: Vec<String> = result["layers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["base".to_string()]);

    if layers.len() <= 1 {
        println!("Only one layer available: {}", current);
        return;
    }

    let idx = layers.iter().position(|l| l == &current).unwrap_or(0);
    let next_idx = if backward {
        if idx == 0 {
            layers.len() - 1
        } else {
            idx - 1
        }
    } else {
        (idx + 1) % layers.len()
    };

    let next_layer = &layers[next_idx];
    match SyncClient::request(
        socket_path,
        "set_layer",
        Some(serde_json::json!({ "layer": next_layer })),
    ) {
        Ok(_) => println!("Layer: {} → {}", current, next_layer),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

// ── Watch (event stream) ───────────────────────────────────────────────────────

fn run_watch(socket_path: &std::path::Path, json_output: bool) {
    let mut stream = match connect_with_timeout(socket_path, 0) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: cannot connect to daemon: {}", e);
            std::process::exit(1);
        }
    };
    // No read timeout — block indefinitely waiting for events.
    let _ = stream.set_read_timeout(None);

    let subscribe_req = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "subscribe", "params": {}
    });
    if let Err(e) = write_frame(&mut stream, &subscribe_req) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    loop {
        match decode_frame(&mut stream) {
            Ok(frame) => {
                if frame.get("id").and_then(|v| v.as_null()) == Some(())
                    && frame.get("method").and_then(|v| v.as_str()) == Some("event")
                {
                    let params = &frame["params"];
                    if json_output {
                        println!("{}", serde_json::to_string(params).unwrap_or_default());
                    } else {
                        print_event_human(params);
                    }
                }
                // responses (e.g. subscribe ack) are ignored
            }
            Err(e) => {
                eprintln!("Disconnected: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn print_event_human(params: &serde_json::Value) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let ts_ms = params["timestamp_ms"].as_u64().unwrap_or(0);
    let dt = if ts_ms > 0 {
        let secs = ts_ms / 1000;
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(secs);
        if elapsed == 0 {
            "now".to_string()
        } else {
            format!("{}s ago", elapsed)
        }
    } else {
        "?".to_string()
    };

    let event_type = params["type"].as_str().unwrap_or("unknown");
    match event_type {
        "trigger_activated" => {
            let id = params["trigger_id"].as_str().unwrap_or("?");
            println!("  ● {} fired  ({})", id, dt);
        }
        "trigger_deactivated" => {
            let id = params["trigger_id"].as_str().unwrap_or("?");
            println!("  ○ {} stopped  ({})", id, dt);
        }
        "layer_changed" => {
            let from = params["from"].as_str().unwrap_or("?");
            let to = params["to"].as_str().unwrap_or("?");
            println!("  ⇒ layer: {} → {}  ({})", from, to, dt);
        }
        "enabled_changed" => {
            let enabled = params["enabled"].as_bool().unwrap_or(false);
            println!(
                "  {} wayclick {}  ({})",
                if enabled { "▶" } else { "■" },
                if enabled { "enabled" } else { "disabled" },
                dt
            );
        }
        "config_reloaded" => {
            println!("  ↺ config reloaded  ({})", dt);
        }
        _ => {
            println!("  ? {}  ({})", event_type, dt);
        }
    }
}

// ── Waybar output ──────────────────────────────────────────────────────────────

fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{}h {}m", h, m)
    }
}

fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
    }
}

fn waybar_json_owned(
    text: &str,
    tooltip: &str,
    classes: &[String],
    percentage: u8,
) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "tooltip": tooltip,
        "class": classes,
        "percentage": percentage,
    })
}

fn waybar_disconnected() -> serde_json::Value {
    waybar_json_owned(
        "󰟸 ✗",
        "wayclick: not running",
        &["disconnected".to_string()],
        0,
    )
}

/// Per-trigger summary used by the waybar state machine.
#[derive(Clone, Default)]
struct TriggerInfo {
    id: String,
    mode: String,
    action: String,
    active: bool,
    activate_count: u64,
    user_enabled: bool,
}

/// Full cached waybar state, updated by events and full re-fetches.
#[derive(Default)]
struct WaybarState {
    enabled: bool,
    dry_run: bool,
    layer: String,
    uptime_secs: u64,
    backend: String,
    trigger_count: usize,
    triggers: Vec<TriggerInfo>,
    layers: Vec<String>,
    /// `Some((id, until))` while the triggering flash CSS class is active.
    flash: Option<(String, Instant)>,
}

fn build_waybar_output(state: &WaybarState, format: &WaybarFormat) -> serde_json::Value {
    if !state.enabled {
        return build_waybar_disabled(state, format);
    }

    let layer_display = titlecase(&state.layer);
    let active_count = state.triggers.iter().filter(|t| t.active).count();

    // Build bar text
    let text = match format {
        WaybarFormat::Minimal => "󰟸".to_string(),
        WaybarFormat::Normal => format!("󰟸 {}", layer_display),
        WaybarFormat::Verbose => {
            if active_count > 0 {
                format!("󰟸 {} ⚡{}", layer_display, active_count)
            } else {
                format!("󰟸 {}", layer_display)
            }
        }
        WaybarFormat::Triggers => {
            let dots: String = state
                .triggers
                .iter()
                .take(8)
                .map(|t| if t.active { '●' } else { '○' })
                .collect();
            if dots.is_empty() {
                "󰟸".to_string()
            } else {
                format!("󰟸 {}", dots)
            }
        }
    };

    let tooltip = build_rich_tooltip(state);

    let mut classes: Vec<String> = vec!["enabled".to_string()];
    classes.push(format!(
        "layer-{}",
        state.layer.to_lowercase().replace(' ', "-")
    ));
    if active_count > 0 {
        classes.push("active".to_string());
    } else {
        classes.push("idle".to_string());
    }
    if state.dry_run {
        classes.push("dry-run".to_string());
    }
    // Flash classes: added briefly on trigger_activated
    if let Some((ref trigger_id, until)) = state.flash {
        if Instant::now() < until {
            classes.push("triggering".to_string());
            classes.push(format!(
                "trigger-{}",
                trigger_id.to_lowercase().replace([' ', '_'], "-")
            ));
        }
    }

    waybar_json_owned(&text, &tooltip, &classes, 100)
}

fn build_waybar_disabled(state: &WaybarState, format: &WaybarFormat) -> serde_json::Value {
    let text = match format {
        WaybarFormat::Minimal => "󰟸".to_string(),
        _ => "󰟸 Off".to_string(),
    };
    let tooltip = build_rich_tooltip(state);
    waybar_json_owned(&text, &tooltip, &["disabled".to_string()], 0)
}

fn build_rich_tooltip(state: &WaybarState) -> String {
    let status_line = if !state.enabled {
        format!("wayclick ── disabled ── {}", titlecase(&state.layer))
    } else if state.dry_run {
        format!("wayclick ── dry-run ── {}", titlecase(&state.layer))
    } else {
        format!("wayclick ── enabled ── {}", titlecase(&state.layer))
    };

    let active_count = state.triggers.iter().filter(|t| t.active).count();
    let trigger_header = format!(
        " Triggers  {} total · {} active",
        state.trigger_count, active_count
    );

    let mut trigger_lines = vec![
        "─────────────────────────────────────".to_string(),
        trigger_header,
    ];

    if state.triggers.is_empty() {
        trigger_lines.push(" (no triggers configured)".to_string());
    } else {
        trigger_lines.push(String::new());
        for t in &state.triggers {
            let dot = if t.active {
                "●"
            } else if !t.user_enabled {
                "✗"
            } else {
                "○"
            };
            trigger_lines.push(format!(
                "  {} {:<20} {:<8} {:<16} {:>4}×",
                dot, t.id, t.mode, t.action, t.activate_count,
            ));
        }
    }

    let layer_list = if state.layers.is_empty() {
        state.layer.clone()
    } else {
        state
            .layers
            .iter()
            .map(|l| {
                if l == &state.layer {
                    format!("{}·", l)
                } else {
                    l.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    };

    let footer = format!(
        "{}  ·  uptime {}",
        state.backend,
        format_uptime(state.uptime_secs)
    );

    let mut parts = vec![status_line];
    parts.extend(trigger_lines);
    if !state.layers.is_empty() {
        parts.push(String::new());
        parts.push("─────────────────────────────────────".to_string());
        parts.push(format!(" Layers  {}", layer_list));
    }
    parts.push(String::new());
    parts.push("─────────────────────────────────────".to_string());
    parts.push(format!(" {}", footer));

    parts.join("\n")
}

/// Populate state from a `status` response result.
fn apply_status(state: &mut WaybarState, result: &serde_json::Value) {
    state.enabled = result["enabled"].as_bool().unwrap_or(false);
    state.dry_run = result["dry_run"].as_bool().unwrap_or(false);
    state.layer = result["layer"]
        .as_str()
        .or_else(|| result["current_layer"].as_str())
        .unwrap_or("base")
        .to_string();
    state.uptime_secs = result["uptime_secs"].as_u64().unwrap_or(0);
    state.backend = result["backend"].as_str().unwrap_or("?").to_string();
    state.trigger_count = result["trigger_count"].as_u64().unwrap_or(0) as usize;

    // Update active flags on our cached triggers list
    let active_ids: Vec<&str> = result["active_triggers"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    for t in &mut state.triggers {
        t.active = active_ids.contains(&t.id.as_str());
    }
}

/// Populate state from a `list_triggers` response result.
fn apply_triggers(state: &mut WaybarState, result: &serde_json::Value) {
    if let Some(arr) = result.as_array() {
        state.triggers = arr
            .iter()
            .filter_map(|t| {
                let id = t["id"].as_str()?.to_string();
                Some(TriggerInfo {
                    id,
                    mode: t["mode"].as_str().unwrap_or("toggle").to_string(),
                    action: t["action_type"].as_str().unwrap_or("?").to_string(),
                    active: t["active"].as_bool().unwrap_or(false),
                    activate_count: t["activate_count"].as_u64().unwrap_or(0),
                    user_enabled: t["user_enabled"].as_bool().unwrap_or(true),
                })
            })
            .collect();
        state.trigger_count = state.triggers.len();
    }
}

/// Populate state from a `list_layers` response result.
fn apply_layers(state: &mut WaybarState, result: &serde_json::Value) {
    if let Some(arr) = result["layers"].as_array() {
        state.layers = arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    if let Some(current) = result["current"].as_str() {
        state.layer = current.to_string();
    }
}

fn run_waybar(
    socket_path: &std::path::Path,
    continuous: bool,
    _interval: u64,
    format: &WaybarFormat,
    flash_ms: u64,
) {
    if continuous {
        run_waybar_event_driven(socket_path, format, flash_ms);
    } else {
        let output = fetch_waybar_snapshot(socket_path, format);
        println!("{}", output);
    }
}

/// One-shot waybar output: fetch status (and triggers for rich tooltip).
fn fetch_waybar_snapshot(socket_path: &std::path::Path, format: &WaybarFormat) -> String {
    let mut state = WaybarState::default();
    state.layer = "base".to_string();

    if let Ok(resp) = SyncClient::request(socket_path, "status", None) {
        if let Some(result) = resp.get("result") {
            apply_status(&mut state, result);
        } else {
            return serde_json::to_string(&waybar_disconnected()).unwrap_or_default();
        }
    } else {
        return serde_json::to_string(&waybar_disconnected()).unwrap_or_default();
    }

    // Best-effort: get triggers and layers for rich tooltip.
    if let Ok(resp) = SyncClient::request(socket_path, "list_triggers", None) {
        if let Some(result) = resp.get("result") {
            apply_triggers(&mut state, result);
        }
    }
    if let Ok(resp) = SyncClient::request(socket_path, "list_layers", None) {
        if let Some(result) = resp.get("result") {
            apply_layers(&mut state, result);
        }
    }

    serde_json::to_string(&build_waybar_output(&state, format)).unwrap_or_default()
}

/// Returns true if an IPC error represents a read timeout.
fn is_timeout_err(e: &wayclick_ipc_client::frame::IpcError) -> bool {
    use wayclick_ipc_client::frame::IpcError;
    if let IpcError::Io(io_err) = e {
        io_err.kind() == io::ErrorKind::WouldBlock || io_err.kind() == io::ErrorKind::TimedOut
    } else {
        false
    }
}

/// Event-driven continuous waybar mode.
/// Subscribes to IPC events, updates cached state on each event, emits JSON lines on stdout.
/// Reconnects automatically with exponential backoff on socket disconnect.
fn run_waybar_event_driven(socket_path: &std::path::Path, format: &WaybarFormat, flash_ms: u64) {
    let flash_duration = Duration::from_millis(flash_ms.max(50));
    let mut backoff = Duration::from_millis(500);

    loop {
        match waybar_streaming_session(socket_path, format, flash_duration) {
            Ok(()) => break,
            Err(_) => {
                // Emit disconnected state so waybar reflects daemon absence.
                println!(
                    "{}",
                    serde_json::to_string(&waybar_disconnected()).unwrap_or_default()
                );
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(8));
            }
        }
    }
}

/// One streaming session: connect → init → event loop. Returns Err on disconnect.
fn waybar_streaming_session(
    socket_path: &std::path::Path,
    format: &WaybarFormat,
    flash_duration: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    use wayclick_ipc_client::frame::IpcError;

    // Use a generous timeout for the initialization phase.
    let mut stream = connect_with_timeout(socket_path, 5000)?;

    let mut next_id: u64 = 1;
    let mut state = WaybarState::default();
    state.layer = "base".to_string();

    // Subscribe to all events and fetch initial full state in a single burst.
    let id_sub = next_id;
    next_id += 1;
    let id_status = next_id;
    next_id += 1;
    let id_triggers = next_id;
    next_id += 1;
    let id_layers = next_id;
    next_id += 1;

    for (id, method) in [
        (id_sub, "subscribe"),
        (id_status, "status"),
        (id_triggers, "list_triggers"),
        (id_layers, "list_layers"),
    ] {
        write_frame(
            &mut stream,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": serde_json::Value::Null,
            }),
        )?;
    }

    // Collect all 4 responses (push events during init are discarded).
    let mut pending = 4usize;
    while pending > 0 {
        let frame = decode_frame(&mut stream)?;
        let fid = frame.get("id").and_then(|v| v.as_u64());
        if let Some(id) = fid {
            if id == id_status {
                if let Some(r) = frame.get("result") {
                    apply_status(&mut state, r);
                }
            } else if id == id_triggers {
                if let Some(r) = frame.get("result") {
                    apply_triggers(&mut state, r);
                }
            } else if id == id_layers {
                if let Some(r) = frame.get("result") {
                    apply_layers(&mut state, r);
                }
            }
            pending -= 1;
        }
        // push events (id=null) during init: ignore, state will reflect via later events
    }

    // Emit initial state.
    println!(
        "{}",
        serde_json::to_string(&build_waybar_output(&state, format)).unwrap_or_default()
    );

    // Main event loop: drive read timeouts for flash management.
    loop {
        // Set socket timeout: either until flash expires, or a keepalive period.
        let timeout = match &state.flash {
            Some((_, until)) => {
                let remaining = until.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    state.flash = None;
                    println!(
                        "{}",
                        serde_json::to_string(&build_waybar_output(&state, format))
                            .unwrap_or_default()
                    );
                    Duration::from_secs(30)
                } else {
                    remaining + Duration::from_millis(5)
                }
            }
            None => Duration::from_secs(30),
        };

        if let Err(e) = stream.set_read_timeout(Some(timeout)) {
            return Err(e.into());
        }

        match decode_frame(&mut stream) {
            Ok(frame) => {
                let is_event = frame.get("id").is_some_and(|v| v.is_null())
                    && frame.get("method").and_then(|v| v.as_str()) == Some("event");

                if !is_event {
                    continue; // stray response frame, ignore
                }

                let params = &frame["params"];
                let event_type = params["type"].as_str().unwrap_or("");

                let mut emit = true;
                match event_type {
                    "trigger_activated" => {
                        let tid = params["trigger_id"].as_str().unwrap_or("").to_string();
                        for t in &mut state.triggers {
                            if t.id == tid {
                                t.active = true;
                                t.activate_count += 1;
                            }
                        }
                        state.flash = Some((tid, Instant::now() + flash_duration));
                    }
                    "trigger_deactivated" => {
                        let tid = params["trigger_id"].as_str().unwrap_or("");
                        for t in &mut state.triggers {
                            if t.id == tid {
                                t.active = false;
                            }
                        }
                        if state.flash.as_ref().is_some_and(|(id, _)| id == tid) {
                            state.flash = None;
                        }
                    }
                    "layer_changed" => {
                        if let Some(to) = params["to"].as_str() {
                            state.layer = to.to_string();
                        }
                    }
                    "enabled_changed" => {
                        state.enabled = params["enabled"].as_bool().unwrap_or(false);
                    }
                    "config_reloaded" => {
                        // Re-fetch triggers and layers on the same connection.
                        let id_st2 = next_id;
                        next_id += 1;
                        let id_tr2 = next_id;
                        next_id += 1;
                        let id_la2 = next_id;
                        next_id += 1;
                        for (id, method) in [
                            (id_st2, "status"),
                            (id_tr2, "list_triggers"),
                            (id_la2, "list_layers"),
                        ] {
                            write_frame(
                                &mut stream,
                                &serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "method": method,
                                    "params": serde_json::Value::Null,
                                }),
                            )?;
                        }
                        // Set a short timeout to collect these responses.
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut pending = 3usize;
                        while pending > 0 {
                            match decode_frame(&mut stream) {
                                Ok(f) => {
                                    if let Some(id) = f.get("id").and_then(|v| v.as_u64()) {
                                        if id == id_st2 {
                                            if let Some(r) = f.get("result") {
                                                apply_status(&mut state, r);
                                            }
                                        } else if id == id_tr2 {
                                            if let Some(r) = f.get("result") {
                                                apply_triggers(&mut state, r);
                                            }
                                        } else if id == id_la2 {
                                            if let Some(r) = f.get("result") {
                                                apply_layers(&mut state, r);
                                            }
                                        }
                                        pending -= 1;
                                    }
                                }
                                Err(e) if is_timeout_err(&e) => break,
                                Err(e) => return Err(e.into()),
                            }
                        }
                    }
                    _ => {
                        emit = false;
                    }
                }

                if emit {
                    println!(
                        "{}",
                        serde_json::to_string(&build_waybar_output(&state, format))
                            .unwrap_or_default()
                    );
                }
            }
            Err(ref e) if is_timeout_err(e) => {
                // Flash expired: clear and emit clean state.
                if state.flash.is_some() {
                    state.flash = None;
                    println!(
                        "{}",
                        serde_json::to_string(&build_waybar_output(&state, format))
                            .unwrap_or_default()
                    );
                }
                // Otherwise it was a keepalive timeout — just loop.
            }
            Err(IpcError::ConnectionClosed) => {
                return Err("connection closed".into());
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    }
}

// Allow the UnixStream from connect_with_timeout to be used with set_read_timeout directly.
impl WaybarState {
    // silence dead_code: Instant is used in flash tuple
    #[allow(dead_code)]
    fn flash_active(&self) -> bool {
        self.flash
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() < *until)
    }
}
