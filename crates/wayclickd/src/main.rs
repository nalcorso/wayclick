// SPDX-License-Identifier: MIT
use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use wayclick_core::config::{effective_socket_path, Config};
use wayclick_core::config_watcher::ConfigWatcher;
use wayclick_core::engine::{with_engine_events, Engine};
use wayclick_core::evdev_monitor::EvdevMonitor;
use wayclick_core::event_bus::EventBus;
use wayclick_core::focus_tracker::FocusTracker;
use wayclick_core::input_backend::{InputBackend, LoggingBackend};
use wayclick_core::ipc::IpcServer;
use wayclick_core::logger::{LogLevel, Logger};
use wayclick_core::lua_api::load_config;
use wayclick_core::uinput_backend::UinputBackend;

#[derive(Parser)]
#[command(
    name = "wayclickd",
    about = "Wayclick programmable mouse automation daemon"
)]
#[command(version)]
struct Cli {
    /// Path to init.lua config file
    #[arg(long, default_value_t = default_config_path())]
    config: String,

    /// Validate config and exit (exit 0 = OK, 1 = error)
    #[arg(long)]
    check_config: Option<String>,

    /// Check /dev/uinput and /dev/input access, then exit
    #[arg(long)]
    check_permissions: bool,

    /// Override config: force dry_run = true
    #[arg(long)]
    dry_run: bool,

    /// Start with automation enabled (default: disabled)
    #[arg(long)]
    enable: bool,

    /// Log level: trace, debug, info, warn, error
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Emit structured JSON log lines
    #[arg(long)]
    log_json: bool,

    /// Override IPC socket path
    #[arg(long)]
    socket: Option<String>,
}

fn default_config_path() -> String {
    if let Ok(v) = std::env::var("WAYCLICK_CONFIG") {
        return v;
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    format!("{}/.config/wayclick/init.lua", home)
}

fn main() {
    let cli = Cli::parse();

    // Handle --check-config
    if let Some(check_path) = &cli.check_config {
        let log_level = LogLevel::from_str_level(&cli.log_level).unwrap_or(LogLevel::Info);
        let logger = Arc::new(Logger::new(100, log_level, cli.log_json));
        let path = PathBuf::from(check_path);
        match load_config(&path, &logger) {
            Ok(config) => {
                println!(
                    "Config OK: {} triggers, {} device bindings",
                    config.triggers.len(),
                    config.device_bindings.len()
                );
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Config error: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Handle --check-permissions
    if cli.check_permissions {
        check_permissions(&cli.config);
        return;
    }

    // Normal daemon startup
    let log_level = LogLevel::from_str_level(&cli.log_level).unwrap_or(LogLevel::Info);
    let logger = Arc::new(Logger::new(512, log_level, cli.log_json));
    logger.info("wayclickd starting");

    // Load config
    let config_path = PathBuf::from(&cli.config);
    let mut config = match load_config(&config_path, &logger) {
        Ok(c) => c,
        Err(e) => {
            logger.error(format!("Failed to load config: {}", e));
            logger.info("Starting with empty config (dry-run mode)");
            Config::default()
        }
    };

    // Apply --dry-run override
    if cli.dry_run {
        config.options.dry_run = true;
    }

    // Apply --socket override
    if let Some(socket) = &cli.socket {
        config.options.socket_path = Some(socket.clone());
    }

    let socket_path = effective_socket_path(&config);

    // Create shared event bus
    let event_bus = Arc::new(EventBus::new());

    // Start focus tracker (auto-detects Hyprland, Sway, or falls back to None).
    let focus_tracker = FocusTracker::start(event_bus.clone(), logger.clone());

    // Create backend: UinputBackend for real mode, LoggingBackend for dry-run
    let backend: Arc<dyn wayclick_core::input_backend::InputBackend> = if config.options.dry_run {
        logger.info("Starting in dry-run mode (LoggingBackend)");
        Arc::new(LoggingBackend::new(logger.clone()))
    } else {
        let mut uinput = UinputBackend::new(logger.clone());
        match uinput.init() {
            Ok(()) => {
                logger.info("UinputBackend initialized successfully");
                let uinput_arc: Arc<dyn wayclick_core::input_backend::InputBackend> =
                    Arc::new(uinput);

                // Try to upgrade the pointer half to zwlr_virtual_pointer_v1. The
                // uinput tablet-axis pointer cannot address multi-monitor layouts
                // correctly — a single absolute-axis device can't represent a
                // non-rectangular union of monitors, so clicks land on the wrong
                // output. The Wayland pointer takes global compositor pixels via
                // motion_absolute and is the only way to make `click_at` land on
                // the correct monitor.
                let initial_extents = compute_extents_from_focus_tracker(&focus_tracker);
                logger.info(format!(
                    "Initial pointer extents: {}×{} (from focus tracker)",
                    initial_extents.width, initial_extents.height
                ));
                let pointer: Arc<dyn wayclick_core::pointer_backend::PointerBackend> =
                    match wayclick_core::wlr_pointer::WlrVirtualPointer::try_new(
                        logger.clone(),
                        initial_extents,
                    ) {
                        Ok(wlr) => {
                            logger.info(
                                "Pointer backend: zwlr_virtual_pointer_v1 (Wayland native, \
                                 multi-monitor accurate).",
                            );
                            let wlr = Arc::new(wlr);
                            spawn_pointer_extents_refresher(
                                wlr.clone(),
                                focus_tracker.clone(),
                                logger.clone(),
                            );
                            wlr
                        }
                        Err(e) => {
                            logger.warn(format!(
                                "zwlr_virtual_pointer_v1 unavailable ({}). Falling back to \
                                 uinput pointer; click_at may not address the correct monitor \
                                 on multi-output layouts.",
                                e
                            ));
                            Arc::new(
                                wayclick_core::pointer_backend::InputBackendPointerAdapter::new(
                                    uinput_arc.clone(),
                                ),
                            )
                        }
                    };

                Arc::new(wayclick_core::pointer_backend::RoutedBackend::new(
                    uinput_arc, pointer,
                ))
            }
            Err(e) => {
                logger.warn(format!(
                    "Failed to init UinputBackend: {}. Falling back to dry-run mode.",
                    e
                ));
                config.options.dry_run = true;
                Arc::new(LoggingBackend::new(logger.clone()))
            }
        }
    };

    // Create engine
    let engine = Arc::new(Mutex::new(Engine::new(
        config.clone(),
        backend.clone(),
        logger.clone(),
        event_bus.clone(),
        cli.config.clone(),
    )));

    // Wire the focus tracker into the engine so monitor-aware ClickAt and
    // MouseMoveAbsolute can resolve their `monitor` field to global coords.
    with_engine_events(&engine, |eng| eng.set_focus_tracker(focus_tracker.clone()));

    // Enable if --enable flag set
    if cli.enable {
        with_engine_events(&engine, |eng| eng.set_enabled(true));
    }

    // Start IPC server
    let ipc_server = match IpcServer::new(
        socket_path.clone(),
        engine.clone(),
        logger.clone(),
        event_bus.clone(),
        Some(focus_tracker.clone()),
    ) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            logger.error(format!("Failed to start IPC server: {}", e));
            std::process::exit(1);
        }
    };
    let ipc_shutdown = ipc_server.shutdown_flag();

    let ipc_server_clone = ipc_server.clone();
    let ipc_handle = thread::spawn(move || {
        ipc_server_clone.run();
    });

    // Start EvdevMonitor with forwarding backend
    let mut evdev_monitor = EvdevMonitor::new(engine.clone(), logger.clone());
    evdev_monitor.set_event_bus(event_bus.clone());
    evdev_monitor.set_backend(backend);
    evdev_monitor.configure(config.device_bindings.clone());
    evdev_monitor.start();

    // Start ConfigWatcher
    let config_path_clone = config_path.clone();
    let logger_clone = logger.clone();
    let engine_clone = engine.clone();
    let _cli_config = cli.config.clone();
    let dry_run_override = cli.dry_run;

    let mut config_watcher = ConfigWatcher::new(
        config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf(),
        logger.clone(),
    );
    config_watcher.start(move || {
        logger_clone.info("Reloading config...");
        match load_config(&config_path_clone, &logger_clone) {
            Ok(mut new_config) => {
                if dry_run_override {
                    new_config.options.dry_run = true;
                }
                with_engine_events(&engine_clone, |eng| eng.apply_config(new_config));
                logger_clone.info("Config reloaded successfully");
            }
            Err(e) => {
                logger_clone.error(format!("Config reload failed: {}", e));
            }
        }
    });

    // Signal handling
    let shutdown = Arc::new(AtomicBool::new(false));
    let reload_signal = Arc::new(AtomicBool::new(false));

    // Install signal handlers using nix
    let _ = signal_hook(shutdown.clone(), reload_signal.clone());

    logger.info(format!("wayclickd ready. Socket: {:?}", socket_path));

    // Main loop
    while !shutdown.load(Ordering::Relaxed) {
        // Check for SIGHUP reload
        if reload_signal.swap(false, Ordering::Relaxed) {
            logger.info("SIGHUP received, reloading config...");
            match load_config(&config_path, &logger) {
                Ok(mut new_config) => {
                    if dry_run_override {
                        new_config.options.dry_run = true;
                    }
                    // Update engine (publishes config_reloaded event after lock is released)
                    with_engine_events(&engine, |eng| eng.apply_config(new_config.clone()));
                    // Restart evdev monitor with new bindings
                    evdev_monitor.stop();
                    evdev_monitor.configure(new_config.device_bindings);
                    evdev_monitor.start();
                    logger.info("Config reloaded via SIGHUP");
                }
                Err(e) => {
                    logger.error(format!("SIGHUP reload failed: {}", e));
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Graceful shutdown
    logger.info("Shutting down...");
    config_watcher.stop();
    evdev_monitor.stop();
    focus_tracker.stop();
    ipc_shutdown.store(true, Ordering::Relaxed);
    let _ = ipc_handle.join();
    logger.info("wayclickd stopped");
}

fn signal_hook(
    shutdown: Arc<AtomicBool>,
    reload: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let shutdown_clone = shutdown.clone();
    let reload_clone = reload.clone();
    thread::spawn(move || {
        use nix::sys::signal::{SigSet, Signal};
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGINT);
        mask.add(Signal::SIGTERM);
        mask.add(Signal::SIGHUP);
        mask.thread_block().ok();
        loop {
            match mask.wait() {
                Ok(Signal::SIGHUP) => {
                    reload_clone.store(true, Ordering::Relaxed);
                }
                Ok(_) => {
                    shutdown_clone.store(true, Ordering::Relaxed);
                    break;
                }
                Err(_) => continue,
            }
        }
    });
    Ok(())
}

fn check_permissions(config_path: &str) {
    println!("Permission Check");
    println!("────────────────────────────────");

    // Check /dev/uinput
    let _uinput_ok = std::fs::metadata("/dev/uinput")
        .map(|_m| {
            // Check if writable by checking mode
            true
        })
        .unwrap_or(false);
    if std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
    {
        println!("/dev/uinput          ✓ writable");
    } else {
        println!("/dev/uinput          ✗ not writable");
    }

    // Check /dev/input/event*
    let input_readable = std::fs::read_dir("/dev/input")
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("event"))
                .any(|e| std::fs::File::open(e.path()).is_ok())
        })
        .unwrap_or(false);
    if input_readable {
        println!("/dev/input/event*    ✓ readable");
    } else {
        println!("/dev/input/event*    ✗ not readable (add user to 'input' group)");
    }

    // Check config
    let config = PathBuf::from(config_path);
    if config.exists() {
        println!("Lua config           ✓ found at {}", config.display());
    } else {
        println!("Lua config           ✗ not found at {}", config.display());
    }

    // Check IPC socket dir
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(&runtime_dir);
        if p.exists() {
            println!("IPC socket dir       ✓ {} writable", runtime_dir);
        } else {
            println!("IPC socket dir       ✗ {} does not exist", runtime_dir);
        }
    } else {
        println!("IPC socket dir       ✗ XDG_RUNTIME_DIR not set");
    }

    println!("────────────────────────────────");
}

/// Compute the smallest bounding box `(width, height)` that covers all
/// monitor logical rectangles (`x + logical_width`, `y + logical_height`).
///
/// This is what `zwlr_virtual_pointer_v1::motion_absolute` needs as its
/// `x_extent`/`y_extent`: the compositor maps `(x/x_extent, y/y_extent)`
/// across its desktop layout, so the extents must equal the layout span.
/// Falls back to a sane default (1920×1080) when the focus tracker can't
/// report monitor info — better than `0×0` which would divide-by-zero on
/// the compositor side.
fn compute_extents_from_focus_tracker(
    ft: &FocusTracker,
) -> wayclick_core::wlr_pointer::DesktopExtents {
    let Some(mons) = ft.monitors() else {
        return wayclick_core::wlr_pointer::DesktopExtents::new(1920, 1080);
    };
    let mut max_x = 0i32;
    let mut max_y = 0i32;
    for m in &mons {
        max_x = max_x.max(m.x.saturating_add(m.logical_width));
        max_y = max_y.max(m.y.saturating_add(m.logical_height));
    }
    if max_x <= 0 || max_y <= 0 {
        return wayclick_core::wlr_pointer::DesktopExtents::new(1920, 1080);
    }
    wayclick_core::wlr_pointer::DesktopExtents::new(max_x as u32, max_y as u32)
}

/// Spawn a background thread that periodically re-queries the focus tracker
/// for monitor layout and pushes the union rect into the Wayland pointer.
/// This keeps `motion_absolute` scaling accurate after hot-plug, resolution
/// changes, or display rotations without requiring a daemon restart.
///
/// Poll-based (every 2s) rather than event-driven because the focus tracker
/// doesn't yet expose layout-change subscriptions; the cost of one Hyprland
/// socket round-trip every 2s is negligible.
fn spawn_pointer_extents_refresher(
    pointer: Arc<wayclick_core::wlr_pointer::WlrVirtualPointer>,
    focus_tracker: Arc<FocusTracker>,
    logger: Arc<Logger>,
) {
    thread::Builder::new()
        .name("wayclickd-pointer-extents".into())
        .spawn(move || {
            let mut last = compute_extents_from_focus_tracker(&focus_tracker);
            loop {
                thread::sleep(Duration::from_secs(2));
                let current = compute_extents_from_focus_tracker(&focus_tracker);
                if current != last {
                    logger.info(format!(
                        "Pointer extents updated: {}×{} -> {}×{} (display layout change)",
                        last.width, last.height, current.width, current.height
                    ));
                    pointer.set_extents(current);
                    last = current;
                }
            }
        })
        .expect("failed to spawn pointer-extents refresher thread");
}
