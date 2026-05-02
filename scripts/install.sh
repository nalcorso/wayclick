#!/usr/bin/env bash
# install.sh — Install wayclick system-wide
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DRY_RUN=0

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--dry-run]"
            echo ""
            echo "Options:"
            echo "  --dry-run    Show what would be installed without making changes"
            echo "  -h, --help   Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Helper to execute commands conditionally
run_cmd() {
    local prefix="$1"
    shift
    if [ $DRY_RUN -eq 1 ]; then
        echo "$prefix (dry-run): $*"
    else
        "$@"
    fi
}

echo "=== wayclick installation ==="
if [ $DRY_RUN -eq 1 ]; then
    echo "(DRY-RUN MODE: No changes will be made)"
fi
echo ""

# Check prerequisites
echo "Checking prerequisites..."
if ! command -v cargo &>/dev/null; then
    echo "✗ Error: cargo not found. Please install Rust 1.85+ from https://rustup.rs" >&2
    exit 1
fi

if ! command -v sudo &>/dev/null; then
    echo "✗ Error: sudo not found. This installation requires sudo for system-wide setup." >&2
    exit 1
fi
echo "✓ Prerequisites found"
echo ""

# Build release binaries
echo "Building release binaries (this may take a few minutes)..."
cd "$PROJECT_DIR"
run_cmd "Would build" cargo build --workspace --release

# Install binaries
echo ""
echo "Installing binaries..."
for bin in wayclickd wayclickctl wayclick-tui wayclick-evdev-dump; do
    if [ ! -f "target/release/$bin" ]; then
        echo "✗ Error: target/release/$bin not found. Build may have failed." >&2
        exit 1
    fi
    
    # Try ~/.cargo/bin first, fall back to /usr/local/bin
    if [ -d "${HOME}/.cargo/bin" ]; then
        run_cmd "→ Install $bin to" cp "target/release/$bin" "${HOME}/.cargo/bin/"
    else
        run_cmd "→ Install $bin to /usr/local/bin (requires sudo)" sudo cp "target/release/$bin" /usr/local/bin/
    fi
done
echo "✓ Binaries staged for installation"

# Set up groups and udev rules
echo ""
echo "Setting up permissions (requires sudo)..."
run_cmd "→ Create wayclick group" sudo groupadd -f wayclick
run_cmd "→ Add $USER to wayclick group" sudo usermod -aG wayclick "$USER"
run_cmd "→ Add $USER to input group" sudo usermod -aG input "$USER"
echo "✓ Groups configured"

run_cmd "→ Install udev rules to /etc/udev/rules.d/" sudo cp "$PROJECT_DIR/udev/99-wayclick.rules" /etc/udev/rules.d/
run_cmd "→ Reload udev" sudo udevadm control --reload
run_cmd "→ Trigger udev" sudo udevadm trigger
echo "✓ udev rules installed"

# Install systemd service
echo ""
echo "Installing systemd service..."
run_cmd "→ Create systemd user directory" mkdir -p "${HOME}/.config/systemd/user"
run_cmd "→ Install service file" cp "$PROJECT_DIR/systemd/wayclickd.service" "${HOME}/.config/systemd/user/"
run_cmd "→ Reload systemd" systemctl --user daemon-reload
echo "✓ systemd service installed"

# Install example config
echo ""
echo "Setting up configuration..."
CONFIG_DIR="${HOME}/.config/wayclick"
run_cmd "→ Create config directory" mkdir -p "$CONFIG_DIR/lua"
if [ ! -f "$CONFIG_DIR/init.lua" ]; then
    run_cmd "→ Install example config" cp "$PROJECT_DIR/config/init.lua" "$CONFIG_DIR/init.lua"
    echo "✓ Example config installed to $CONFIG_DIR/init.lua"
else
    echo "✓ Config already exists at $CONFIG_DIR/init.lua (skipped)"
fi

# Summary
echo ""
echo "=== Installation $([ $DRY_RUN -eq 1 ] && echo "[DRY-RUN]" || echo "complete") ==="
echo ""
if [ $DRY_RUN -eq 0 ]; then
    echo "Next steps:"
    echo "  1. Activate group membership:"
    echo "     • Log out and back in, OR"
    echo "     • Run: newgrp wayclick"
    echo "  2. Start the daemon:"
    echo "     systemctl --user enable --now wayclickd"
    echo "  3. Verify installation:"
    echo "     wayclickctl status"
    echo "  4. View real-time status:"
    echo "     wayclick-tui"
    echo ""
    echo "For detailed setup and troubleshooting, see:"
    echo "  • docs/QUICKSTART.md"
    echo "  • docs/PERMISSIONS.md"
    echo "  • docs/TROUBLESHOOTING.md"
else
    echo "To proceed with installation, run:"
    echo "  $0"
fi
