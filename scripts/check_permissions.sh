#!/usr/bin/env bash
# check_permissions.sh — Verify wayclick permission requirements
set -euo pipefail

echo "=== wayclick permission check ==="
echo ""

ERRORS=0

# Check /dev/uinput
if [ -w /dev/uinput ]; then
    echo "✓ /dev/uinput is writable"
else
    echo "✗ /dev/uinput is NOT writable"
    echo "  Fix: sudo groupadd -f wayclick && sudo usermod -aG wayclick \$USER"
    echo "       Then install udev rules and re-login"
    ERRORS=$((ERRORS + 1))
fi

# Check /dev/input/event*
INPUT_READABLE=0
for dev in /dev/input/event*; do
    if [ -r "$dev" ]; then
        INPUT_READABLE=1
        break
    fi
done
if [ "$INPUT_READABLE" -eq 1 ]; then
    echo "✓ /dev/input/event* devices are readable"
else
    echo "✗ /dev/input/event* devices are NOT readable"
    echo "  Fix: sudo usermod -aG input \$USER"
    ERRORS=$((ERRORS + 1))
fi

# Check groups
if id -nG "$USER" | grep -qw wayclick; then
    echo "✓ User is in 'wayclick' group"
else
    echo "✗ User is NOT in 'wayclick' group"
    echo "  Fix: sudo usermod -aG wayclick \$USER"
    ERRORS=$((ERRORS + 1))
fi

if id -nG "$USER" | grep -qw input; then
    echo "✓ User is in 'input' group"
else
    echo "✗ User is NOT in 'input' group"
    echo "  Fix: sudo usermod -aG input \$USER"
    ERRORS=$((ERRORS + 1))
fi

# Check udev rules
if [ -f /etc/udev/rules.d/99-wayclick.rules ]; then
    echo "✓ udev rules installed"
else
    echo "✗ udev rules NOT installed"
    echo "  Fix: sudo cp udev/99-wayclick.rules /etc/udev/rules.d/"
    ERRORS=$((ERRORS + 1))
fi

# Check XDG_RUNTIME_DIR
if [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -d "$XDG_RUNTIME_DIR" ]; then
    echo "✓ XDG_RUNTIME_DIR is set ($XDG_RUNTIME_DIR)"
else
    echo "✗ XDG_RUNTIME_DIR is not set or does not exist"
    ERRORS=$((ERRORS + 1))
fi

# Check config
CONFIG="${HOME}/.config/wayclick/init.lua"
if [ -f "$CONFIG" ]; then
    echo "✓ Config found at $CONFIG"
else
    echo "✗ Config NOT found at $CONFIG"
    echo "  Fix: cp config/init.lua ~/.config/wayclick/init.lua"
    ERRORS=$((ERRORS + 1))
fi

echo ""
if [ "$ERRORS" -eq 0 ]; then
    echo "All checks passed! ✓"
else
    echo "$ERRORS check(s) failed. See above for fixes."
    exit 1
fi
