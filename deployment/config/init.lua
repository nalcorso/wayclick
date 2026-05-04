-- wayclick default configuration
-- MB5 (forward/extra button) toggles a left-click auto-clicker on and off.
-- Scroll wheel up/down fires a left-click per notch (ARPG scroll-to-click).
-- See examples/ for more advanced configs (morse code, macros, etc).

wayclick.set_options({
  dry_run = false,
  log_capacity = 512,
})

wayclick.register_trigger({
  id = "auto_clicker",
  name = "Auto Clicker",
  mode = "toggle",
  cooldown_ms = 300,
  action = wayclick.auto_click({
    button = "left",
    interval_ms = 8,
    jitter_ms = 0,
    hold_ms = 2,
  }),
})

wayclick.register_trigger({
  id = "left_click",
  name = "Scroll Click",
  mode = "oneshot",
  action = wayclick.click({ button = "left" }),
})

wayclick.bind_device({
  name = "Logitech USB Receiver Mouse",
  exclusive = true,
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
    { scroll = "up",   trigger = "left_click" },
    { scroll = "down", trigger = "left_click" },
  },
})
