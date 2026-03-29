# Building Wayclick

## Prerequisites

- Rust 1.75+ (via [rustup](https://rustup.rs))
- Linux with kernel support for uinput and evdev
- `gcc` or `clang` (for Lua vendored build)

## Build

```sh
# Debug build
cargo build --workspace

# Release build
cargo build --workspace --release
```

Binaries are placed in `target/debug/` or `target/release/`:

- `wayclickd` — the daemon
- `wayclickctl` — the CLI control tool
- `wayclick-tui` — the TUI dashboard
- `wayclick-evdev-dump` — the device diagnostic tool

## Install

```sh
# Install to ~/.cargo/bin
cargo install --path crates/wayclickd
cargo install --path crates/wayclickctl
cargo install --path crates/wayclick-tui
cargo install --path crates/wayclick-evdev-dump
```

## Tests

```sh
# Run all tests (unit + integration)
cargo test --workspace

# Run only core library tests
cargo test -p wayclick-core

# Run integration tests
cargo test -p wayclick-tests

# Run uinput integration tests (requires /dev/uinput write access)
cargo test -p wayclick-core --features integration
```

## Development Setup

```sh
# Quick dev setup — grants permissions and installs config
./scripts/dev.sh

# Check permissions
wayclickd --check-permissions

# Validate config
wayclickd --check-config ~/.config/wayclick/init.lua
```

## Config Location

The default config path is `~/.config/wayclick/init.lua`. Override with:

```sh
wayclickd --config /path/to/init.lua
# or
export WAYCLICK_CONFIG=/path/to/init.lua
```

## Cross-Compilation

Wayclick is Linux-only (requires uinput/evdev). The vendored Lua build should
work on any Linux target with a C compiler.

```sh
# For a specific target
cargo build --target x86_64-unknown-linux-gnu --release
```

## Fuzz Testing

```sh
cd fuzz
cargo +nightly fuzz run fuzz_config_loader
cargo +nightly fuzz run fuzz_ipc_frame
cargo +nightly fuzz run fuzz_device_match
```
