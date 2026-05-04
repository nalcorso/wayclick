#!/usr/bin/env bash
# dev.sh — Quick development setup for wayclick
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "=== wayclick development setup ==="

# Create config directory
CONFIG_DIR="${HOME}/.config/wayclick"
mkdir -p "$CONFIG_DIR"

# Install example config if not present
if [ ! -f "$CONFIG_DIR/init.lua" ]; then
    cp "$PROJECT_DIR/deployment/config/init.lua" "$CONFIG_DIR/init.lua"
    echo "✓ Installed example config to $CONFIG_DIR/init.lua"
else
    echo "✓ Config already exists at $CONFIG_DIR/init.lua"
fi

# Create lua module directory
mkdir -p "$CONFIG_DIR/lua"
echo "✓ Created $CONFIG_DIR/lua/"

# Build the project
echo ""
echo "Building..."
cd "$PROJECT_DIR"
cargo build --workspace
echo "✓ Build successful"

# Check permissions
echo ""
echo "Checking permissions..."
"$PROJECT_DIR/target/debug/wayclickd" --check-permissions

echo ""
echo "=== Setup complete ==="
echo "Run: ./target/debug/wayclickd --dry-run --enable"
