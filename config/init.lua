-- wayclick default configuration
-- MB5 (forward/extra button) toggles a left-click auto-clicker on and off.

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
    interval_ms = 10,
    jitter_ms = 5,
  }),
})

-- Bind MB5 (BTN_EXTRA) on any mouse to toggle the auto-clicker
wayclick.bind_device({
  name = "",          -- matches any device name
  bindings = {
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
