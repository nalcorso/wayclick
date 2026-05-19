-- Example: monitor-aware click_at for a card-activation macro.
--
-- Demonstrates:
--   * wayclick.click_at with the optional `monitor = "DP-2"` field, which
--     interprets (x, y) as monitor-local logical pixels.
--   * Sequence + toggle mode: one press starts the loop, another stops it.
--   * Helper functions to keep coordinate arithmetic readable.
--
-- Layout assumption (Revolution Idle on a 4K monitor named "DP-2"):
--   * 8x4 grid of cards starting at (154, 552), 338px x 386px per cell.
--   * "Activate" button at (2183, 1370).
-- Adjust the constants / monitor name for your own setup.

wayclick.set_options({
  dry_run = false,
  log_capacity = 512,
})

local MONITOR = "DP-2"

local function activate_card()
  return wayclick.sequence({
    actions = {
      wayclick.click_at({
        x = 2183,
        y = 1370,
        button = "left",
        monitor = MONITOR,
        hold_ms = 100,
      }),
      wayclick.delay({ ms = 500 }),
    },
  })
end

local function select_card(i, j)
  return wayclick.sequence({
    actions = {
      wayclick.click_at({
        x = 154 + i * 338,
        y = 552 + j * 386,
        button = "left",
        monitor = MONITOR,
        hold_ms = 100,
      }),
      wayclick.delay({ ms = 500 }),
    },
  })
end

-- Toggle mode: press once to start looping, press again to stop.
-- The Sequence wraps in an implicit loop under toggle/hold modes.
wayclick.register_trigger({
  id = "activate_cards",
  name = "Activate Cards",
  description = "Cycle through The Fool (0,0), The Chariot (0,3), The Devil (1,1)",
  mode = "toggle",
  cooldown_ms = 300,
  action = wayclick.sequence({
    actions = {
      select_card(0, 0), activate_card(),  -- The Fool
      select_card(0, 3), activate_card(),  -- The Chariot
      select_card(1, 1), activate_card(),  -- The Devil
    },
  }),
})

wayclick.bind_device({
  name = "Logitech USB Receiver Mouse",
  exclusive = true,
  bindings = {
    { code = "BTN_EXTRA", trigger = "activate_cards" },
  },
})
