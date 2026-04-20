-- scroll_remap.lua — Remap scroll wheel to left-click (ARPG gaming)
--
-- A common ARPG technique: scroll wheel up/down fires left-click for rapid
-- clicking without wearing out your mouse button. Each scroll notch fires one
-- click; fast scrolling fires multiple clicks per frame.
--
-- Usage:
--   Copy this file to ~/.config/wayclick/init.lua (or require it from yours).
--   Change the device name to match your mouse (run `wayclickctl devices` to list).
--
-- Requirements:
--   - exclusive = true (required for scroll remapping)
--   - Your user must have permission to access /dev/input/event* devices
--     (see docs/PERMISSIONS.md)

wayclick.set_options({
  dry_run = false,
  log_capacity = 512,
})

---------------------------------------------------------------------------
-- Trigger: left-click (oneshot fires once per scroll notch)
---------------------------------------------------------------------------
wayclick.register_trigger({
  id = "left_click",
  mode = "oneshot",
  action = wayclick.click({ button = "left" }),
})

---------------------------------------------------------------------------
-- Device binding: scroll wheel → left-click
--
-- Replace "G Pro" with your mouse name. You can also match by VID:PID:
--   vid = 0x046d, pid = 0xc08b,
--
-- exclusive = true grabs the physical device so scroll events don't reach
-- the game directly. Non-consumed events (mouse movement, unmatched buttons)
-- are forwarded through wayclick's virtual pointer automatically.
---------------------------------------------------------------------------
wayclick.bind_device({
  name = "G Pro",
  exclusive = true,
  bindings = {
    { scroll = "up",   trigger = "left_click" },
    { scroll = "down", trigger = "left_click" },
  },
})
