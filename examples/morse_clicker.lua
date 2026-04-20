-- morse_clicker.lua — Morse code auto-clicker for Revolution Idle
--
-- Clicks mouse morse code for any word. The game awards an achievement
-- for clicking "banana" in morse code using an auto-clicker.
--
-- Usage:
--   Copy this file to ~/.config/wayclick/init.lua (or require it from yours).
--   Press MB4 (BTN_SIDE) to send "banana" in morse code clicks.
--
-- Timing constants are tuned for Revolution Idle's click detection.
-- Adjust DOT_MS, DASH_MS, ELEMENT_GAP, and LETTER_GAP if your game
-- needs different timings.

wayclick.set_options({
  dry_run = false,
  log_capacity = 512,
})

---------------------------------------------------------------------------
-- Timing (milliseconds)
---------------------------------------------------------------------------
local DOT_MS      = 50    -- duration of a dot element
local DASH_MS     = 500   -- duration of a dash element
local ELEMENT_GAP = 150   -- gap between elements within a letter
local LETTER_GAP  = 2000  -- gap between letters

---------------------------------------------------------------------------
-- International Morse Code dictionary (A–Z, 0–9)
---------------------------------------------------------------------------
local MORSE = {
  A = ".-",     B = "-...",   C = "-.-.",   D = "-..",
  E = ".",      F = "..-.",   G = "--.",    H = "....",
  I = "..",     J = ".---",   K = "-.-",    L = ".-..",
  M = "--",     N = "-.",     O = "---",    P = ".--.",
  Q = "--.-",   R = ".-.",    S = "...",    T = "-",
  U = "..-",    V = "...-",   W = ".--",    X = "-..-",
  Y = "-.--",   Z = "--..",

  ["0"] = "-----",  ["1"] = ".----",  ["2"] = "..---",
  ["3"] = "...--",  ["4"] = "....-",  ["5"] = ".....",
  ["6"] = "-....",  ["7"] = "--...",   ["8"] = "---..",
  ["9"] = "----.",
}

---------------------------------------------------------------------------
-- Morse element builders
---------------------------------------------------------------------------
local function click()
  return wayclick.auto_click({ button = "left" })
end

local function dot()
  return wayclick.sequence({ actions = {
    click(), wayclick.delay({ ms = DOT_MS }), click(),
  }})
end

local function dash()
  return wayclick.sequence({ actions = {
    click(), wayclick.delay({ ms = DASH_MS }), click(),
  }})
end

---------------------------------------------------------------------------
-- Build a wayclick action sequence for a single letter
---------------------------------------------------------------------------
local function morse_letter(ch)
  local code = MORSE[ch:upper()]
  if not code then
    error("No morse code for character: " .. ch)
  end

  local actions = {}
  for i = 1, #code do
    if i > 1 then
      table.insert(actions, wayclick.delay({ ms = ELEMENT_GAP }))
    end
    local symbol = code:sub(i, i)
    if symbol == "." then
      table.insert(actions, dot())
    elseif symbol == "-" then
      table.insert(actions, dash())
    end
  end

  return wayclick.sequence({ actions = actions })
end

---------------------------------------------------------------------------
-- Build a wayclick action sequence for an entire word
---------------------------------------------------------------------------
local function morse_word(word)
  local actions = {}
  for i = 1, #word do
    if i > 1 then
      table.insert(actions, wayclick.delay({ ms = LETTER_GAP }))
    end
    table.insert(actions, morse_letter(word:sub(i, i)))
  end
  return wayclick.sequence({ actions = actions })
end

---------------------------------------------------------------------------
-- Register the trigger — spells "banana" in morse code clicks
---------------------------------------------------------------------------
wayclick.register_trigger({
  id = "morse_banana",
  name = "Morse Banana",
  description = "Clicks morse code for 'banana' (Revolution Idle achievement)",
  mode = "oneshot",
  action = morse_word("banana"),
})

wayclick.bind_device({
  name = "",
  bindings = {
    { code = "BTN_SIDE", trigger = "morse_banana" },
  },
})
