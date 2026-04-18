-- wayclick default configuration
-- MB4 (side button) triggers a oneshot morse code macro that clicks "banana".
-- MB5 (forward/extra button) toggles a left-click auto-clicker on and off.

wayclick.set_options({
  dry_run = false,
  log_capacity = 512,
})

-- Morse code helpers
local function click()
  return wayclick.auto_click({ button = "left" })
end

local function dot()
  -- Dot: click, 50ms gap, click
  return wayclick.sequence({ actions = {
    click(), wayclick.delay({ ms = 50 }), click(),
  }})
end

local function dash()
  -- Dash: click, 500ms gap, click
  return wayclick.sequence({ actions = {
    click(), wayclick.delay({ ms = 500 }), click(),
  }})
end

local ELEMENT_GAP = 150   -- ms between elements within a letter
local LETTER_GAP  = 2000  -- ms between letters

-- B: -...
local letter_b = wayclick.sequence({ actions = {
  dash(),
  wayclick.delay({ ms = ELEMENT_GAP }),
  dot(),
  wayclick.delay({ ms = ELEMENT_GAP }),
  dot(),
  wayclick.delay({ ms = ELEMENT_GAP }),
  dot(),
}})

-- A: .-
local letter_a = wayclick.sequence({ actions = {
  dot(),
  wayclick.delay({ ms = ELEMENT_GAP }),
  dash(),
}})

-- N: -.
local letter_n = wayclick.sequence({ actions = {
  dash(),
  wayclick.delay({ ms = ELEMENT_GAP }),
  dot(),
}})

wayclick.register_trigger({
  id = "morse_banana",
  name = "Morse Banana",
  description = "Clicks morse code for 'banana'",
  mode = "oneshot",
  action = wayclick.sequence({ actions = {
    letter_b,
    wayclick.delay({ ms = LETTER_GAP }),
    letter_a,
    wayclick.delay({ ms = LETTER_GAP }),
    letter_n,
    wayclick.delay({ ms = LETTER_GAP }),
    letter_a,
    wayclick.delay({ ms = LETTER_GAP }),
    letter_n,
    wayclick.delay({ ms = LETTER_GAP }),
    letter_a,
  }}),
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

-- Bind MB4 (BTN_SIDE) to morse banana, MB5 (BTN_EXTRA) to auto-clicker
wayclick.bind_device({
  name = "",          -- matches any device name
  bindings = {
    { code = "BTN_SIDE",  trigger = "morse_banana" },
    { code = "BTN_EXTRA", trigger = "auto_clicker" },
  },
})
