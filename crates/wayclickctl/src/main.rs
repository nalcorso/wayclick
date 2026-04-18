use clap::{Parser, Subcommand};
use std::path::PathBuf;
use wayclick_core::config::default_socket_path;
use wayclick_core::ipc::ipc_request;

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
    /// Fire a named trigger
    Trigger {
        /// Trigger ID to fire
        id: String,
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
}

fn main() {
    let cli = Cli::parse();

    let socket_path = match &cli.socket {
        Some(p) => PathBuf::from(p),
        None => default_socket_path(),
    };

    let result = match &cli.command {
        Command::Ping => ipc_request(&socket_path, "ping", None),
        Command::Status => ipc_request(&socket_path, "status", None),
        Command::Toggle => ipc_request(&socket_path, "toggle", None),
        Command::Enable => ipc_request(&socket_path, "enable", None),
        Command::Disable => ipc_request(&socket_path, "disable", None),
        Command::Trigger { id } => {
            ipc_request(
                &socket_path,
                "trigger",
                Some(serde_json::json!({ "id": id, "press": true })),
            )
        }
        Command::List => ipc_request(&socket_path, "list_triggers", None),
        Command::Reload => ipc_request(&socket_path, "reload_config", None),
        Command::Logs { tail } => {
            ipc_request(
                &socket_path,
                "logs_tail",
                Some(serde_json::json!({ "n": tail })),
            )
        }
        Command::Layer { action } => match action {
            LayerAction::Get => ipc_request(&socket_path, "get_layer", None),
            LayerAction::Set { name } => {
                ipc_request(
                    &socket_path,
                    "set_layer",
                    Some(serde_json::json!({ "layer": name })),
                )
            }
        },
    };

    match result {
        Ok(response) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&response).unwrap());
                return;
            }

            // Check for error
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
                    println!("Config:     {}", result["config_path"].as_str().unwrap_or("?"));
                    println!("Layer:      {}", result["layer"].as_str().unwrap_or("base"));
                    println!("Uptime:     {}s", result["uptime_secs"]);
                }
                Command::Toggle => {
                    let enabled = result["enabled"].as_bool().unwrap_or(false);
                    println!("{}", if enabled { "enabled" } else { "disabled" });
                }
                Command::Enable => println!("enabled"),
                Command::Disable => println!("disabled"),
                Command::Trigger { id } => {
                    println!("Triggered: {}", id);
                }
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
                                let status = if active { "ACTIVE" } else { "idle" };
                                println!(
                                    "  {} {} [{}] {}  {}",
                                    if active { "●" } else { "○" },
                                    id,
                                    status,
                                    mode,
                                    action,
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
                },
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

