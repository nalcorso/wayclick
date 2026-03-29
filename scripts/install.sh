#!/usr/bin/env bash
# install.sh — Install wayclick system-wide
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== wayclick installation ==="

# Build release binaries
echo "Building release binaries..."
cd "$PROJECT_DIR"
cargo build --workspace --release

# Install binaries
echo "Installing binaries to ~/.cargo/bin/"
for bin in wayclickd wayclickctl wayclick-tui wayclick-evdev-dump; do
    cp "target/release/$bin" "${HOME}/.cargo/bin/" 2>/dev/null || {
        echo "Note: ~/.cargo/bin/ not found, installing to /usr/local/bin/ (needs sudo)"
        sudo cp "target/release/$bin" /usr/local/bin/
    }
done
echo "✓ Binaries installed"

# Set up groups and udev rules
echo ""
echo "Setting up permissions (requires sudo)..."
sudo groupadd -f wayclick
sudo usermod -aG wayclick "$USER"
sudo usermod -aG input "$USER"
echo "✓ Groups configured"

sudo cp "$PROJECT_DIR/udev/99-wayclick.rules" /etc/udev/rules.d/
sudo udevadm control --reload
sudo udevadm trigger
echo "✓ udev rules installed"

# Install systemd service
mkdir -p "${HOME}/.config/systemd/user"
cp "$PROJECT_DIR/systemd/wayclickd.service" "${HOME}/.config/systemd/user/"
systemctl --user daemon-reload
echo "✓ systemd service installed"

# Install example config
CONFIG_DIR="${HOME}/.config/wayclick"
mkdir -p "$CONFIG_DIR/lua"
if [ ! -f "$CONFIG_DIR/init.lua" ]; then
    cp "$PROJECT_DIR/config/init.lua" "$CONFIG_DIR/init.lua"
    echo "✓ Example config installed"
fi

echo ""
echo "=== Installation complete ==="
echo ""
echo "Next steps:"
echo "  1. Log out and back in (or run: newgrp wayclick)"
echo "  2. Start the daemon: systemctl --user enable --now wayclickd"
echo "  3. Check status: wayclickctl status"
echo "  4. Open the TUI: wayclick-tui"
