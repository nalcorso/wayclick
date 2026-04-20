-- wayclick default configuration
-- MB5 (forward/extra button) toggles a left-click auto-clicker on and off.
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

wayclick.bind_device({
  name = "",          -- matches any device name
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
