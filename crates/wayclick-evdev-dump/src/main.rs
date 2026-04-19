use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use wayclick_core::evdev_source::{self, EvdevSource, InputSource};

#[derive(Parser)]
#[command(
    name = "wayclick-evdev-dump",
    about = "Input device diagnostic tool for wayclick"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all accessible input devices
    List,
    /// Monitor a device and print raw events
    Monitor {
        /// Device path (e.g. /dev/input/event5)
        #[arg(long)]
        device: PathBuf,
    },
    /// Interactive device identification — press a button to see which device it is
    Identify {
        /// Timeout in seconds (default: 10)
        #[arg(long, default_value = "10")]
        timeout: u64,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::List => cmd_list(),
        Commands::Monitor { device } => cmd_monitor(device),
        Commands::Identify { timeout } => cmd_identify(timeout),
    }
}

fn cmd_list() {
    let devices = evdev_source::enumerate_devices();

    if devices.is_empty() {
        eprintln!("No accessible input devices found.");
        eprintln!("Make sure you have read permissions on /dev/input/event*.");
        eprintln!("Try running with sudo or add yourself to the 'input' group.");
        std::process::exit(1);
    }

    println!("{:<30} {:<10} {:<40} PHYS", "PATH", "VID:PID", "NAME");
    println!("{}", "-".repeat(100));

    for dev in &devices {
        println!(
            "{:<30} {:04x}:{:04x}   {:<40} {}",
            dev.path.display(),
            dev.vendor_id,
            dev.product_id,
            dev.name,
            dev.phys,
        );
    }

    println!("\n{} device(s) found", devices.len());
}

fn cmd_monitor(device: PathBuf) {
    println!("Opening device: {}", device.display());

    let mut source = match EvdevSource::open(&device, false) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error opening device: {}", e);
            std::process::exit(1);
        }
    };

    let info = source.device_info();
    println!(
        "Device: {} ({:04x}:{:04x})",
        info.name, info.vendor_id, info.product_id
    );
    println!("Press Ctrl+C to stop.\n");
    println!("{:<10} {:<10} {:<10}", "TYPE", "CODE", "VALUE");
    println!("{}", "-".repeat(30));

    loop {
        match source.poll_events(Duration::from_millis(500)) {
            Ok(events) => {
                for ev in &events {
                    let type_name = match ev.event_type {
                        0x00 => "SYN",
                        0x01 => "KEY",
                        0x02 => "REL",
                        0x03 => "ABS",
                        0x04 => "MSC",
                        _ => "???",
                    };
                    // Only print non-SYN events to reduce noise
                    if ev.event_type != 0x00 {
                        println!("{:<10} {:<10} {:<10}", type_name, ev.code, ev.value);
                    }
                }
            }
            Err(evdev_source::SourceError::Disconnected) => {
                println!("\nDevice disconnected.");
                break;
            }
            Err(e) => {
                eprintln!("\nError: {}", e);
                break;
            }
        }
    }

    source.close();
}

fn cmd_identify(timeout: u64) {
    let devices = evdev_source::enumerate_devices();

    if devices.is_empty() {
        eprintln!("No accessible input devices found.");
        std::process::exit(1);
    }

    println!("Opening {} devices for identification...", devices.len());
    println!(
        "Press any button on the device you want to identify (timeout: {}s)\n",
        timeout
    );

    let mut sources: Vec<EvdevSource> = Vec::new();

    for dev in &devices {
        if let Ok(s) = EvdevSource::open(&dev.path, false) {
            sources.push(s);
        }
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout);

    while std::time::Instant::now() < deadline {
        for source in &mut sources {
            if let Ok(events) = source.poll_events(Duration::from_millis(10)) {
                for ev in &events {
                    if ev.event_type == 0x01 && ev.value == 1 {
                        let info = source.device_info();
                        println!("=== DEVICE IDENTIFIED ===");
                        println!("  Path:    {}", info.path.display());
                        println!("  Name:    {}", info.name);
                        println!("  VID:PID: {:04x}:{:04x}", info.vendor_id, info.product_id);
                        println!("  Phys:    {}", info.phys);
                        println!("  Button:  code={}", ev.code);
                        println!();
                        println!("Lua device match examples:");
                        println!("  wayclick.device {{ name_contains = \"{}\" }}", info.name);
                        println!(
                            "  wayclick.device {{ vid = 0x{:04x}, pid = 0x{:04x} }}",
                            info.vendor_id, info.product_id
                        );

                        // Clean up
                        for s in sources {
                            s.close();
                        }
                        return;
                    }
                }
            }
        }
    }

    println!("Timeout — no button press detected.");

    for s in sources {
        s.close();
    }
}
