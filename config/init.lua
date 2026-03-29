local wc = wayclick

wc.set_options({
  dry_run = false,
  log_capacity = 512,
})

-- Trigger 1: Toggle rapid left-click loop on mouse button 4
wc.register_trigger({
  id = "rapid_fire",
  name = "Rapid Fire",
  mode = "toggle",
  cooldown_ms = 300,
  action = wc.auto_click({
    button = "left",
    interval_ms = 10,
    jitter_ms = 5,
  }),
})

-- Trigger 2: Hold mouse button 5 to send staggered key presses
wc.register_trigger({
  id = "key_matrix",
  name = "Key Matrix",
  mode = "toggle",
  action = wc.parallel({
    actions = {
      wc.key_press({ key = "1", interval_ms = 30000 }),
      wc.key_press({ key = "3", interval_ms = 59000 }),
    },
  }),
})

-- Trigger 3: One-shot burst (fires 5 clicks and stops)
wc.register_trigger({
  id = "burst_fire",
  name = "Burst Fire",
  mode = "oneshot",
  action = wc.auto_click({
    button = "left",
    interval_ms = 50,
    duration_ms = 250,
  }),
})

-- Bind triggers to physical device buttons
wc.bind_device({
  name = "Logitech G Pro Gaming Mouse",
  bindings = {
    { code = "BTN_SIDE",  trigger = "rapid_fire" },
    { code = "BTN_EXTRA", trigger = "key_matrix" },
  },
})
