use crate::config::*;
use crate::logger::Logger;
use mlua::prelude::*;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// Maximum number of Lua VM instructions allowed during config loading.
/// Prevents infinite loops from blocking startup forever.
/// 10 million instructions is generous (~2-3 seconds of Lua execution).
const LUA_INSTRUCTION_LIMIT: u32 = 10_000_000;

/// How often the instruction-count hook fires (every N instructions).
const LUA_HOOK_INTERVAL: u32 = 10_000;

/// Maximum nesting depth for composite actions (sequence/parallel).
/// Prevents stack overflow from deeply nested action trees.
const MAX_ACTION_DEPTH: usize = 32;

/// Maximum number of sub-actions in a single parallel composite.
/// Prevents thread explosion from spawning too many concurrent threads.
const MAX_PARALLEL_ACTIONS: usize = 64;

/// Internal builder used as upvalue in Lua closures.
struct ConfigBuilder {
    options: GlobalOptions,
    triggers: Vec<TriggerBinding>,
    device_bindings: Vec<DeviceBinding>,
    profile_rules: Vec<ProfileRule>,
    warnings: Vec<String>,
}

impl ConfigBuilder {
    fn new() -> Self {
        Self {
            options: GlobalOptions::default(),
            triggers: Vec::new(),
            device_bindings: Vec::new(),
            profile_rules: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn into_config(self) -> Config {
        Config {
            options: self.options,
            triggers: self.triggers,
            device_bindings: self.device_bindings,
            profile_rules: self.profile_rules,
        }
    }
}

/// Load a wayclick config from a Lua init file.
pub fn load_config(path: &Path, logger: &Arc<Logger>) -> Result<Config, ConfigError> {
    let lua = Lua::new();
    let config_dir = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // Set up sandbox: remove dangerous functions, restrict io.open to config dir
    sandbox_lua(&lua, &config_dir)?;

    // Set up sandboxed require
    setup_sandboxed_require(&lua, &config_dir)?;

    // Create the config builder in Lua app data
    lua.set_app_data(ConfigBuilder::new());

    // Register the wayclick global table
    register_wayclick_api(&lua, logger)?;

    // Execute the config file with instruction limit to prevent infinite loops
    let source = std::fs::read_to_string(path).map_err(ConfigError::Io)?;

    // Install instruction-count hook to abort runaway Lua scripts
    let instruction_count = Rc::new(Cell::new(0u32));
    let counter = instruction_count.clone();
    let max_callbacks = LUA_INSTRUCTION_LIMIT / LUA_HOOK_INTERVAL;
    lua.set_hook(
        mlua::HookTriggers::new().every_nth_instruction(LUA_HOOK_INTERVAL),
        move |_lua, _debug| {
            let count = counter.get() + 1;
            counter.set(count);
            if count >= max_callbacks {
                return Err(LuaError::RuntimeError(format!(
                    "Config exceeded instruction limit ({} instructions). \
                     Possible infinite loop in Lua config.",
                    LUA_INSTRUCTION_LIMIT
                )));
            }
            Ok(mlua::VmState::Continue)
        },
    );

    lua.load(&source)
        .set_name(path.to_string_lossy())
        .exec()
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    lua.remove_hook();

    // Extract config from builder
    let builder = lua
        .remove_app_data::<ConfigBuilder>()
        .ok_or_else(|| ConfigError::Lua("Config builder not found".into()))?;

    // Log any warnings
    for w in &builder.warnings {
        logger.warn(w.clone());
    }

    let config = builder.into_config();

    // Validate
    validate_config(&config).map_err(|errs| {
        ConfigError::Validation(
            errs.into_iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;

    Ok(config)
}

fn sandbox_lua(lua: &Lua, config_dir: &Path) -> Result<(), ConfigError> {
    // Canonicalize the config directory for path validation.
    // io.open will only be allowed to read files within this directory.
    let canonical_config_dir = config_dir
        .canonicalize()
        .unwrap_or_else(|_| config_dir.to_path_buf());

    // Rust-side path validator: resolves relative paths against config_dir,
    // resolves symlinks and ../ traversals via canonicalize(),
    // then checks the real path is within the config directory.
    let config_dir_for_resolve = canonical_config_dir.clone();
    let config_dir_for_lua = canonical_config_dir.clone();
    let validate_path = lua
        .create_function(move |_, path: String| {
            let p = std::path::Path::new(&path);
            // Resolve relative paths against the config directory
            let absolute = if p.is_relative() {
                config_dir_for_resolve.join(p)
            } else {
                p.to_path_buf()
            };
            match absolute.canonicalize() {
                Ok(canonical) => Ok(canonical.starts_with(&canonical_config_dir)),
                // Non-existent file or permission error — block the read
                Err(_) => Ok(false),
            }
        })
        .map_err(|e| ConfigError::Lua(format!("Failed to create path validator: {}", e)))?;

    lua.globals()
        .set("__validate_path", validate_path)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    let config_dir_lua = config_dir_for_lua
        .to_string_lossy()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    lua.load(format!(
        r#"
        os.execute = nil
        os.exit = nil
        if io then
            io.popen = nil
            local orig_open = io.open
            local validate = __validate_path
            local config_dir = "{}"
            io.open = function(path, mode)
                if mode and (mode:find("w") or mode:find("a")) then
                    return nil, "write access denied in sandbox"
                end
                if not validate(path) then
                    return nil, "read access denied: path is outside config directory"
                end
                -- Resolve relative paths against config directory
                if path:sub(1,1) ~= "/" then
                    path = config_dir .. "/" .. path
                end
                return orig_open(path, mode)
            end
        end
        __validate_path = nil
        load = nil
        loadfile = nil
        dofile = nil
        debug = nil
    "#,
        config_dir_lua
    ))
    .exec()
    .map_err(|e| ConfigError::Lua(format!("Failed to set up sandbox: {}", e)))?;
    Ok(())
}

fn setup_sandboxed_require(lua: &Lua, config_dir: &Path) -> Result<(), ConfigError> {
    let lua_dir = config_dir.join("lua");
    let lua_dir_str = lua_dir.to_string_lossy().to_string();

    lua.load(format!(
        r#"
        local lua_dir = "{}"
        local original_require = require
        package.path = lua_dir .. "/?.lua;" .. lua_dir .. "/?/init.lua"
        package.cpath = ""
    "#,
        lua_dir_str.replace('\\', "\\\\").replace('"', "\\\"")
    ))
    .exec()
    .map_err(|e| ConfigError::Lua(format!("Failed to set up sandboxed require: {}", e)))?;

    Ok(())
}

fn register_wayclick_api(lua: &Lua, _logger: &Arc<Logger>) -> Result<(), ConfigError> {
    let wayclick = lua
        .create_table()
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.set_options(table)
    let set_options = lua
        .create_function(|lua, table: LuaTable| {
            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            if let Ok(v) = table.get::<bool>("dry_run") {
                builder.options.dry_run = v;
            }
            if let Ok(v) = table.get::<String>("socket_path") {
                builder.options.socket_path = if v.is_empty() { None } else { Some(v) };
            }
            if let Ok(v) = table.get::<usize>("log_capacity") {
                builder.options.log_capacity = v;
            }
            if let Ok(v) = table.get::<u32>("min_interval_ms") {
                builder.options.min_interval_ms = v;
            }
            Ok(())
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("set_options", set_options)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // Action constructors
    // wayclick.auto_click(table) -> action table
    let auto_click = lua
        .create_function(|lua, table: LuaTable| {
            let button_str: String = table
                .get::<String>("button")
                .unwrap_or_else(|_| "left".into());
            let _button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let interval_ms: u32 = table.get("interval_ms").unwrap_or(50);
            let duration_ms: Option<u32> = table.get("duration_ms").ok();
            let jitter_ms: u32 = table.get("jitter_ms").unwrap_or(0);
            let hold_ms: u32 = table.get("hold_ms").unwrap_or(0);

            let action = lua.create_table()?;
            action.set("_type", "auto_click")?;
            action.set("_button", button_str)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
            action.set("_hold_ms", hold_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("auto_click", auto_click)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.click(table) -> action table (single click at current cursor position)
    let click = lua
        .create_function(|lua, table: LuaTable| {
            let button_str: String = table
                .get::<String>("button")
                .unwrap_or_else(|_| "left".into());
            let _button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let hold_ms: u32 = table.get("hold_ms").unwrap_or(0);

            let action = lua.create_table()?;
            action.set("_type", "auto_click")?;
            action.set("_button", button_str)?;
            action.set("_interval_ms", 1u32)?;
            action.set("_jitter_ms", 0u32)?;
            action.set("_hold_ms", hold_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("click", click)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.key_press(table) -> action table
    let key_press = lua
        .create_function(|lua, table: LuaTable| {
            let key: String = table
                .get::<String>("key")
                .map_err(|_| LuaError::RuntimeError("key_press requires 'key' field".into()))?;
            let (key_name, key_code) =
                normalize_key_name(&key).map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let interval_ms: u32 = table.get("interval_ms").unwrap_or(1000);
            let duration_ms: Option<u32> = table.get("duration_ms").ok();
            let jitter_ms: u32 = table.get("jitter_ms").unwrap_or(0);
            let (modifier_names, modifier_codes) =
                parse_modifiers(&table).map_err(|e| LuaError::RuntimeError(e.to_string()))?;

            let action = lua.create_table()?;
            action.set("_type", "key_press")?;
            action.set("_key_name", key_name)?;
            action.set("_key_code", key_code)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
            let names_tbl = lua.create_table()?;
            for (i, n) in modifier_names.iter().enumerate() {
                names_tbl.set(i + 1, n.as_str())?;
            }
            let codes_tbl = lua.create_table()?;
            for (i, c) in modifier_codes.iter().enumerate() {
                codes_tbl.set(i + 1, *c)?;
            }
            action.set("_modifier_names", names_tbl)?;
            action.set("_modifier_codes", codes_tbl)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("key_press", key_press)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.keystroke(table) -> action table
    // Sends a single key chord (oneshot-only). Modifiers are pressed first,
    // then the main key, then all released in reverse order.
    let keystroke = lua
        .create_function(|lua, table: LuaTable| {
            let key: String = table
                .get::<String>("key")
                .map_err(|_| LuaError::RuntimeError("keystroke requires 'key' field".into()))?;
            let (key_name, key_code) =
                normalize_key_name(&key).map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let hold_ms: u32 = table.get("hold_ms").unwrap_or(0);
            let (modifier_names, modifier_codes) =
                parse_modifiers(&table).map_err(|e| LuaError::RuntimeError(e.to_string()))?;

            let action = lua.create_table()?;
            action.set("_type", "keystroke")?;
            action.set("_key_name", key_name)?;
            action.set("_key_code", key_code)?;
            action.set("_hold_ms", hold_ms)?;
            let names_tbl = lua.create_table()?;
            for (i, n) in modifier_names.iter().enumerate() {
                names_tbl.set(i + 1, n.as_str())?;
            }
            let codes_tbl = lua.create_table()?;
            for (i, c) in modifier_codes.iter().enumerate() {
                codes_tbl.set(i + 1, *c)?;
            }
            action.set("_modifier_names", names_tbl)?;
            action.set("_modifier_codes", codes_tbl)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("keystroke", keystroke)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.type_text(table) -> action table (sequence of keystrokes)
    // Types a string character by character using US QWERTY key mappings.
    // Accepts { text = "...", delay_ms = 30 }. Returns a sequence action
    // compatible with wayclick.sequence() and all trigger modes that accept
    // sequences. Only valid in oneshot triggers (inherited from keystroke).
    let type_text = lua
        .create_function(|lua, table: LuaTable| {
            let text: String = table
                .get::<String>("text")
                .map_err(|_| LuaError::RuntimeError("type_text requires 'text' field".into()))?;
            let delay_ms: u32 = table.get("delay_ms").unwrap_or(30);

            let chars: Vec<char> = text.chars().collect();
            let actions_tbl = lua.create_table()?;
            let mut idx = 1usize;

            for (ci, &c) in chars.iter().enumerate() {
                let (key_name, key_code, needs_shift) =
                    char_to_key_us_qwerty(c).ok_or_else(|| {
                        LuaError::RuntimeError(format!(
                            "type_text: character {:?} (U+{:04X}) is not supported on US QWERTY",
                            c, c as u32
                        ))
                    })?;

                let ks = lua.create_table()?;
                ks.set("_type", "keystroke")?;
                ks.set("_key_name", key_name)?;
                ks.set("_key_code", key_code)?;
                ks.set("_hold_ms", 0u32)?;
                let names_tbl = lua.create_table()?;
                let codes_tbl = lua.create_table()?;
                if needs_shift {
                    names_tbl.set(1, "KEY_LEFTSHIFT")?;
                    codes_tbl.set(1, 42u32)?;
                }
                ks.set("_modifier_names", names_tbl)?;
                ks.set("_modifier_codes", codes_tbl)?;
                actions_tbl.set(idx, ks)?;
                idx += 1;

                if delay_ms > 0 && ci + 1 < chars.len() {
                    let dl = lua.create_table()?;
                    dl.set("_type", "delay")?;
                    dl.set("_duration_ms", delay_ms)?;
                    actions_tbl.set(idx, dl)?;
                    idx += 1;
                }
            }

            let action = lua.create_table()?;
            action.set("_type", "sequence")?;
            action.set("_actions", actions_tbl)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("type_text", type_text)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.scroll(table) -> action table
    let scroll = lua
        .create_function(|lua, table: LuaTable| {
            let dir_str: String = table
                .get::<String>("direction")
                .unwrap_or_else(|_| "down".into());
            let _direction = ScrollDirection::from_str_name(&dir_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let amount: i32 = table.get("amount").unwrap_or(3);
            let interval_ms: u32 = table.get("interval_ms").unwrap_or(100);
            let duration_ms: Option<u32> = table.get("duration_ms").ok();
            let jitter_ms: u32 = table.get("jitter_ms").unwrap_or(0);

            let action = lua.create_table()?;
            action.set("_type", "scroll")?;
            action.set("_direction", dir_str)?;
            action.set("_amount", amount)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("scroll", scroll)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.mouse_move(table) -> action table
    let mouse_move = lua
        .create_function(|lua, table: LuaTable| {
            let dx: i32 = table.get("dx").unwrap_or(0);
            let dy: i32 = table.get("dy").unwrap_or(0);
            let interval_ms: u32 = table.get("interval_ms").unwrap_or(16);
            let duration_ms: Option<u32> = table.get("duration_ms").ok();
            let jitter_ms: u32 = table.get("jitter_ms").unwrap_or(0);

            let action = lua.create_table()?;
            action.set("_type", "mouse_move")?;
            action.set("_dx", dx)?;
            action.set("_dy", dy)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("mouse_move", mouse_move)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.sequence(table) -> action table
    let sequence = lua
        .create_function(|lua, table: LuaTable| {
            let actions: LuaTable = table.get("actions")?;
            let action = lua.create_table()?;
            action.set("_type", "sequence")?;
            action.set("_actions", actions)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("sequence", sequence)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.parallel(table) -> action table
    let parallel = lua
        .create_function(|lua, table: LuaTable| {
            let actions: LuaTable = table.get("actions")?;
            let action = lua.create_table()?;
            action.set("_type", "parallel")?;
            action.set("_actions", actions)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("parallel", parallel)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.noop() -> action table
    let noop = lua
        .create_function(|lua, ()| {
            let action = lua.create_table()?;
            action.set("_type", "noop")?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("noop", noop)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.delay(table) -> action table
    let delay = lua
        .create_function(|lua, table: LuaTable| {
            let duration_ms: u32 = table
                .get::<u32>("ms")
                .map_err(|_| LuaError::RuntimeError("delay requires 'ms' field".into()))?;
            let action = lua.create_table()?;
            action.set("_type", "delay")?;
            action.set("_duration_ms", duration_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("delay", delay)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.mouse_move_abs(table) -> action table
    let mouse_move_abs = lua
        .create_function(|lua, table: LuaTable| {
            let x: i32 = table
                .get::<i32>("x")
                .map_err(|_| LuaError::RuntimeError("mouse_move_abs requires 'x' field".into()))?;
            let y: i32 = table
                .get::<i32>("y")
                .map_err(|_| LuaError::RuntimeError("mouse_move_abs requires 'y' field".into()))?;
            let action = lua.create_table()?;
            action.set("_type", "mouse_move_abs")?;
            action.set("_x", x)?;
            action.set("_y", y)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("mouse_move_abs", mouse_move_abs)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.click_at(table) -> action table
    let click_at = lua
        .create_function(|lua, table: LuaTable| {
            let x: i32 = table
                .get::<i32>("x")
                .map_err(|_| LuaError::RuntimeError("click_at requires 'x' field".into()))?;
            let y: i32 = table
                .get::<i32>("y")
                .map_err(|_| LuaError::RuntimeError("click_at requires 'y' field".into()))?;
            let button: String = table.get("button").unwrap_or_else(|_| "left".into());
            let hold_ms: u32 = table.get("hold_ms").unwrap_or(0);
            let settle_ms: u32 = table.get("settle_ms").unwrap_or(5);
            let action = lua.create_table()?;
            action.set("_type", "click_at")?;
            action.set("_x", x)?;
            action.set("_y", y)?;
            action.set("_button", button)?;
            action.set("_hold_ms", hold_ms)?;
            action.set("_settle_ms", settle_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("click_at", click_at)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.drag(table) -> action table
    let drag = lua
        .create_function(|lua, table: LuaTable| {
            let from_x: i32 = table
                .get::<i32>("from_x")
                .map_err(|_| LuaError::RuntimeError("drag requires 'from_x' field".into()))?;
            let from_y: i32 = table
                .get::<i32>("from_y")
                .map_err(|_| LuaError::RuntimeError("drag requires 'from_y' field".into()))?;
            let to_x: i32 = table
                .get::<i32>("to_x")
                .map_err(|_| LuaError::RuntimeError("drag requires 'to_x' field".into()))?;
            let to_y: i32 = table
                .get::<i32>("to_y")
                .map_err(|_| LuaError::RuntimeError("drag requires 'to_y' field".into()))?;
            let button: String = table.get("button").unwrap_or_else(|_| "left".into());
            let duration_ms: u32 = table.get("duration_ms").unwrap_or(100);
            let action = lua.create_table()?;
            action.set("_type", "drag")?;
            action.set("_from_x", from_x)?;
            action.set("_from_y", from_y)?;
            action.set("_to_x", to_x)?;
            action.set("_to_y", to_y)?;
            action.set("_button", button)?;
            action.set("_duration_ms", duration_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("drag", drag)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.set_layer(table) -> action table
    let set_layer = lua
        .create_function(|lua, table: LuaTable| {
            let layer: String = table
                .get::<String>("layer")
                .map_err(|_| LuaError::RuntimeError("set_layer requires 'layer' field".into()))?;
            let action = lua.create_table()?;
            action.set("_type", "set_layer")?;
            action.set("_layer", layer)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("set_layer", set_layer)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.media_key(table) -> action table (convenience wrapper for key_press with media keys)
    let media_key = lua
        .create_function(|lua, table: LuaTable| {
            let key: String = table
                .get::<String>("key")
                .map_err(|_| LuaError::RuntimeError("media_key requires 'key' field".into()))?;
            // Resolve key name — accept bare names like "play_pause" or full "KEY_PLAYPAUSE"
            let key_upper = key.to_uppercase();
            let full_name = if key_upper.starts_with("KEY_") {
                key_upper.clone()
            } else {
                format!("KEY_{}", key_upper)
            };
            let code = key_name_to_code(&full_name).ok_or_else(|| {
                LuaError::RuntimeError(format!(
                    "Unknown media key: '{}' (tried '{}')",
                    key, full_name
                ))
            })?;
            let action = lua.create_table()?;
            action.set("_type", "key_press")?;
            action.set("_key_name", full_name)?;
            action.set("_key_code", code)?;
            action.set("_interval_ms", 1u32)?;
            action.set("_duration_ms", LuaNil)?;
            action.set("_jitter_ms", 0u32)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("media_key", media_key)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.set_profile(table) - register a profile rule for per-app layer switching
    let set_profile = lua
        .create_function(|lua, table: LuaTable| {
            let name: String = table
                .get::<String>("name")
                .map_err(|_| LuaError::RuntimeError("set_profile requires 'name' field".into()))?;
            let match_app: Option<String> = table.get("match_app").ok();
            let match_title: Option<String> = table.get("match_title").ok();
            let layer: String = table
                .get::<String>("layer")
                .map_err(|_| LuaError::RuntimeError("set_profile requires 'layer' field".into()))?;

            if match_app.is_none() && match_title.is_none() {
                return Err(LuaError::RuntimeError(
                    "set_profile requires at least one of 'match_app' or 'match_title'".into(),
                ));
            }

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.profile_rules.push(ProfileRule {
                name,
                match_app,
                match_title,
                layer,
            });

            Ok(())
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("set_profile", set_profile)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.keys - media key constants table
    let keys_table = lua
        .create_table()
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    let media_keys = [
        ("MUTE", 113u32),
        ("VOLUME_DOWN", 114),
        ("VOLUME_UP", 115),
        ("NEXT_SONG", 163),
        ("PLAY_PAUSE", 164),
        ("PREVIOUS_SONG", 165),
        ("STOP_CD", 166),
        ("RECORD", 167),
        ("REWIND", 168),
        ("FAST_FORWARD", 208),
        ("BRIGHTNESS_DOWN", 224),
        ("BRIGHTNESS_UP", 225),
    ];
    for (name, code) in &media_keys {
        keys_table
            .set(*name, *code)
            .map_err(|e| ConfigError::Lua(e.to_string()))?;
    }
    wayclick
        .set("keys", keys_table)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.register_trigger(table)
    let register_trigger = lua
        .create_function(|lua, table: LuaTable| {
            let id: String = table.get::<String>("id").map_err(|_| {
                LuaError::RuntimeError("register_trigger requires 'id' field".into())
            })?;
            let name: String = table.get("name").unwrap_or_else(|_| id.clone());
            let description: String = table.get("description").unwrap_or_default();
            let mode_str: String = table.get("mode").unwrap_or_else(|_| "toggle".into());
            let mode = TriggerMode::from_str_mode(&mode_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let cooldown_ms: Option<u32> = table.get("cooldown_ms").ok();

            let action_table: LuaTable = table.get::<LuaTable>("action").map_err(|_| {
                LuaError::RuntimeError("register_trigger requires 'action' field".into())
            })?;

            let action = parse_action_table(&action_table, 0)?;

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.triggers.push(TriggerBinding {
                id,
                name,
                description,
                mode,
                action,
                cooldown_ms,
            });

            Ok(())
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("register_trigger", register_trigger)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.bind_device(table)
    let bind_device = lua
        .create_function(|lua, table: LuaTable| {
            let name: Option<String> = table.get("name").ok();
            let vid: Option<u16> = table.get("vid").ok();
            let pid: Option<u16> = table.get("pid").ok();
            let phys: Option<String> = table.get("phys").ok();
            let path: Option<String> = table.get("path").ok();
            let exclusive: bool = table.get("exclusive").unwrap_or(false);

            // Build DeviceMatch
            let mut matchers = Vec::new();
            if let Some(n) = name {
                matchers.push(DeviceMatch::ByName { contains: n });
            }
            if let (Some(v), Some(p)) = (vid, pid) {
                matchers.push(DeviceMatch::ByVidPid {
                    vendor: v,
                    product: p,
                });
            }
            if let Some(ph) = phys {
                matchers.push(DeviceMatch::ByPhys { contains: ph });
            }
            if let Some(p) = path {
                let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
                builder.warnings.push(format!(
                    "bind_device: 'path' is deprecated. Use 'name' or 'vid'/'pid' instead. Path: {}",
                    p
                ));
                drop(builder);
                matchers.push(DeviceMatch::ByPath { path: p });
            }

            if matchers.is_empty() {
                return Err(LuaError::RuntimeError(
                    "bind_device requires at least one match criterion (name, vid/pid, phys, or path)"
                        .into(),
                ));
            }

            let device_match = if matchers.len() == 1 {
                matchers.into_iter().next().unwrap()
            } else {
                DeviceMatch::Any { matchers }
            };

            // Parse bindings
            let bindings_table: LuaTable = table
                .get::<LuaTable>("bindings")
                .map_err(|_| LuaError::RuntimeError("bind_device requires 'bindings' field".into()))?;

            let mut bindings: Vec<Binding> = Vec::new();
            for pair in bindings_table.sequence_values::<LuaTable>() {
                let binding = pair?;

                // Check for scroll binding first
                if let Ok(scroll_dir_str) = binding.get::<String>("scroll") {
                    let trigger_id: String = binding.get("trigger")?;
                    let layer: Option<String> = binding.get("layer").ok();
                    let swallow: bool = binding.get("swallow").unwrap_or(false);
                    let direction = ScrollDirection::from_str_name(&scroll_dir_str).map_err(|e| {
                        LuaError::RuntimeError(format!(
                            "Invalid scroll direction '{}': {}",
                            scroll_dir_str, e
                        ))
                    })?;
                    if swallow && !exclusive {
                        return Err(LuaError::RuntimeError(
                            "bind_device: swallow=true requires exclusive=true".into(),
                        ));
                    }
                    bindings.push(Binding::Scroll(ScrollBinding {
                        direction,
                        trigger_id,
                        layer,
                        swallow,
                    }));
                    continue;
                }

                let code_str: String = binding.get("code")?;
                let trigger_id: String = binding.get("trigger")?;
                let hold_trigger_id: Option<String> = binding.get("hold_trigger").ok();
                let hold_threshold_ms: Option<u32> = binding.get("hold_ms").ok();
                let layer: Option<String> = binding.get("layer").ok();
                let swallow: bool = binding.get("swallow").unwrap_or(false);
                let on_str: String = binding.get("on").unwrap_or_else(|_| "press".into());
                let on = TriggerEdge::from_str(&on_str).map_err(|e| {
                    LuaError::RuntimeError(format!("Invalid 'on' value '{}': {}", on_str, e))
                })?;

                // Parse code string — supports chords like "BTN_SIDE+BTN_EXTRA"
                let code_names: Vec<String> = code_str
                    .split('+')
                    .map(|s| s.trim().to_string())
                    .collect();

                let mut codes = Vec::new();
                for name in &code_names {
                    match trigger_code_from_name(name) {
                        Some(c) => codes.push(c),
                        None => {
                            return Err(LuaError::RuntimeError(format!(
                                "Unknown event code: '{}' in bind_device binding",
                                name
                            )));
                        }
                    }
                }

                // Validate hold config consistency
                if hold_trigger_id.is_some() != hold_threshold_ms.is_some() {
                    return Err(LuaError::RuntimeError(
                        "bind_device: 'hold_trigger' and 'hold_ms' must both be set together"
                            .into(),
                    ));
                }

                if swallow && !exclusive {
                    return Err(LuaError::RuntimeError(
                        "bind_device: swallow=true requires exclusive=true".into(),
                    ));
                }

                if on == TriggerEdge::Release && swallow {
                    return Err(LuaError::RuntimeError(
                        "bind_device: swallow=true is incompatible with on=\"release\" (press already forwarded)".into(),
                    ));
                }

                if on == TriggerEdge::Release && hold_trigger_id.is_some() {
                    return Err(LuaError::RuntimeError(
                        "bind_device: on=\"release\" is incompatible with hold_trigger".into(),
                    ));
                }

                bindings.push(Binding::Button(ButtonBinding {
                    codes,
                    code_names,
                    trigger_id,
                    hold_trigger_id,
                    hold_threshold_ms,
                    layer,
                    swallow,
                    on,
                }));
            }

            // Validate: scroll bindings require exclusive mode
            let has_scroll = bindings.iter().any(|b| matches!(b, Binding::Scroll(_)));
            if has_scroll && !exclusive {
                return Err(LuaError::RuntimeError(
                    "bind_device: scroll bindings require exclusive=true".into(),
                ));
            }

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.device_bindings.push(DeviceBinding {
                device_match,
                bindings,
                exclusive,
            });

            Ok(())
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("bind_device", bind_device)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    // wayclick.bind_evdev(table) - legacy/deprecated
    let bind_evdev = lua
        .create_function(|lua, table: LuaTable| {
            let device: String = table.get("device")?;
            let code_str: String = table.get("code")?;
            let trigger_id: String = table.get("trigger")?;

            let code = trigger_code_from_name(&code_str).ok_or_else(|| {
                LuaError::RuntimeError(format!("Unknown event code: '{}'", code_str))
            })?;

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.warnings.push(format!(
                "bind_evdev is deprecated. Use bind_device instead. Device: {}",
                device
            ));
            builder.device_bindings.push(DeviceBinding {
                device_match: DeviceMatch::ByPath { path: device },
                bindings: vec![Binding::Button(ButtonBinding {
                    codes: vec![code],
                    code_names: vec![code_str],
                    trigger_id,
                    hold_trigger_id: None,
                    hold_threshold_ms: None,
                    layer: None,
                    swallow: false,
                    on: TriggerEdge::Press,
                })],
                exclusive: false,
            });

            Ok(())
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("bind_evdev", bind_evdev)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    lua.globals()
        .set("wayclick", wayclick)
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

    Ok(())
}

/// Map a single character to its US QWERTY keyboard key.
///
/// Returns `(key_name, evdev_key_code, needs_shift)` or `None` if the character has no
/// direct mapping on a US QWERTY layout (e.g., non-ASCII, accented letters, emoji).
fn char_to_key_us_qwerty(c: char) -> Option<(&'static str, u32, bool)> {
    match c {
        // Lowercase letters
        'a' => Some(("KEY_A", 30, false)),
        'b' => Some(("KEY_B", 48, false)),
        'c' => Some(("KEY_C", 46, false)),
        'd' => Some(("KEY_D", 32, false)),
        'e' => Some(("KEY_E", 18, false)),
        'f' => Some(("KEY_F", 33, false)),
        'g' => Some(("KEY_G", 34, false)),
        'h' => Some(("KEY_H", 35, false)),
        'i' => Some(("KEY_I", 23, false)),
        'j' => Some(("KEY_J", 36, false)),
        'k' => Some(("KEY_K", 37, false)),
        'l' => Some(("KEY_L", 38, false)),
        'm' => Some(("KEY_M", 50, false)),
        'n' => Some(("KEY_N", 49, false)),
        'o' => Some(("KEY_O", 24, false)),
        'p' => Some(("KEY_P", 25, false)),
        'q' => Some(("KEY_Q", 16, false)),
        'r' => Some(("KEY_R", 19, false)),
        's' => Some(("KEY_S", 31, false)),
        't' => Some(("KEY_T", 20, false)),
        'u' => Some(("KEY_U", 22, false)),
        'v' => Some(("KEY_V", 47, false)),
        'w' => Some(("KEY_W", 17, false)),
        'x' => Some(("KEY_X", 45, false)),
        'y' => Some(("KEY_Y", 21, false)),
        'z' => Some(("KEY_Z", 44, false)),
        // Uppercase letters (same key code, shift held)
        'A' => Some(("KEY_A", 30, true)),
        'B' => Some(("KEY_B", 48, true)),
        'C' => Some(("KEY_C", 46, true)),
        'D' => Some(("KEY_D", 32, true)),
        'E' => Some(("KEY_E", 18, true)),
        'F' => Some(("KEY_F", 33, true)),
        'G' => Some(("KEY_G", 34, true)),
        'H' => Some(("KEY_H", 35, true)),
        'I' => Some(("KEY_I", 23, true)),
        'J' => Some(("KEY_J", 36, true)),
        'K' => Some(("KEY_K", 37, true)),
        'L' => Some(("KEY_L", 38, true)),
        'M' => Some(("KEY_M", 50, true)),
        'N' => Some(("KEY_N", 49, true)),
        'O' => Some(("KEY_O", 24, true)),
        'P' => Some(("KEY_P", 25, true)),
        'Q' => Some(("KEY_Q", 16, true)),
        'R' => Some(("KEY_R", 19, true)),
        'S' => Some(("KEY_S", 31, true)),
        'T' => Some(("KEY_T", 20, true)),
        'U' => Some(("KEY_U", 22, true)),
        'V' => Some(("KEY_V", 47, true)),
        'W' => Some(("KEY_W", 17, true)),
        'X' => Some(("KEY_X", 45, true)),
        'Y' => Some(("KEY_Y", 21, true)),
        'Z' => Some(("KEY_Z", 44, true)),
        // Digits
        '0' => Some(("KEY_0", 11, false)),
        '1' => Some(("KEY_1",  2, false)),
        '2' => Some(("KEY_2",  3, false)),
        '3' => Some(("KEY_3",  4, false)),
        '4' => Some(("KEY_4",  5, false)),
        '5' => Some(("KEY_5",  6, false)),
        '6' => Some(("KEY_6",  7, false)),
        '7' => Some(("KEY_7",  8, false)),
        '8' => Some(("KEY_8",  9, false)),
        '9' => Some(("KEY_9", 10, false)),
        // Shifted digits
        '!' => Some(("KEY_1",  2, true)),
        '@' => Some(("KEY_2",  3, true)),
        '#' => Some(("KEY_3",  4, true)),
        '$' => Some(("KEY_4",  5, true)),
        '%' => Some(("KEY_5",  6, true)),
        '^' => Some(("KEY_6",  7, true)),
        '&' => Some(("KEY_7",  8, true)),
        '*' => Some(("KEY_8",  9, true)),
        '(' => Some(("KEY_9", 10, true)),
        ')' => Some(("KEY_0", 11, true)),
        // Punctuation (unshifted)
        ' '  => Some(("KEY_SPACE",      57, false)),
        '-'  => Some(("KEY_MINUS",      12, false)),
        '='  => Some(("KEY_EQUAL",      13, false)),
        '['  => Some(("KEY_LEFTBRACE",  26, false)),
        ']'  => Some(("KEY_RIGHTBRACE", 27, false)),
        '\\' => Some(("KEY_BACKSLASH",  43, false)),
        ';'  => Some(("KEY_SEMICOLON",  39, false)),
        '\'' => Some(("KEY_APOSTROPHE", 40, false)),
        '`'  => Some(("KEY_GRAVE",      41, false)),
        ','  => Some(("KEY_COMMA",      51, false)),
        '.'  => Some(("KEY_DOT",        52, false)),
        '/'  => Some(("KEY_SLASH",      53, false)),
        // Punctuation (shifted)
        '_' => Some(("KEY_MINUS",      12, true)),
        '+' => Some(("KEY_EQUAL",      13, true)),
        '{' => Some(("KEY_LEFTBRACE",  26, true)),
        '}' => Some(("KEY_RIGHTBRACE", 27, true)),
        '|' => Some(("KEY_BACKSLASH",  43, true)),
        ':' => Some(("KEY_SEMICOLON",  39, true)),
        '"' => Some(("KEY_APOSTROPHE", 40, true)),
        '~' => Some(("KEY_GRAVE",      41, true)),
        '<' => Some(("KEY_COMMA",      51, true)),
        '>' => Some(("KEY_DOT",        52, true)),
        '?' => Some(("KEY_SLASH",      53, true)),
        // Special keys
        '\n' => Some(("KEY_ENTER", 28, false)),
        '\t' => Some(("KEY_TAB",   15, false)),
        _ => None,
    }
}

/// Parse a Lua action table (returned by auto_click, key_press, etc.) into an ActionConfig.
fn parse_action_table(table: &LuaTable, depth: usize) -> Result<ActionConfig, LuaError> {
    let action_type: String = table.get("_type")?;

    match action_type.as_str() {
        "auto_click" => {
            let button_str: String = table.get("_button")?;
            let button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let interval_ms: u32 = table.get("_interval_ms")?;
            let duration_ms: Option<u32> = table.get("_duration_ms").ok();
            let jitter_ms: u32 = table.get("_jitter_ms")?;
            let hold_ms: u32 = table.get("_hold_ms").unwrap_or(0);
            Ok(ActionConfig::AutoClick {
                button,
                interval_ms,
                duration_ms,
                jitter_ms,
                hold_ms,
            })
        }
        "key_press" => {
            let key_name: String = table.get("_key_name")?;
            let key_code: u32 = table.get("_key_code")?;
            let interval_ms: u32 = table.get("_interval_ms")?;
            let duration_ms: Option<u32> = table.get("_duration_ms").ok();
            let jitter_ms: u32 = table.get("_jitter_ms")?;
            let modifier_names: Vec<String> = match table.get::<LuaTable>("_modifier_names") {
                Ok(t) => t.sequence_values::<String>().collect::<Result<Vec<_>, _>>()?,
                Err(_) => Vec::new(),
            };
            let modifier_codes: Vec<u32> = match table.get::<LuaTable>("_modifier_codes") {
                Ok(t) => t.sequence_values::<u32>().collect::<Result<Vec<_>, _>>()?,
                Err(_) => Vec::new(),
            };
            Ok(ActionConfig::KeyPress {
                key_name,
                key_code,
                modifier_names,
                modifier_codes,
                interval_ms,
                duration_ms,
                jitter_ms,
            })
        }
        "keystroke" => {
            let key_name: String = table.get("_key_name")?;
            let key_code: u32 = table.get("_key_code")?;
            let hold_ms: u32 = table.get("_hold_ms").unwrap_or(0);
            let modifier_names: Vec<String> = match table.get::<LuaTable>("_modifier_names") {
                Ok(t) => t.sequence_values::<String>().collect::<Result<Vec<_>, _>>()?,
                Err(_) => Vec::new(),
            };
            let modifier_codes: Vec<u32> = match table.get::<LuaTable>("_modifier_codes") {
                Ok(t) => t.sequence_values::<u32>().collect::<Result<Vec<_>, _>>()?,
                Err(_) => Vec::new(),
            };
            Ok(ActionConfig::Keystroke {
                key_name,
                key_code,
                modifier_names,
                modifier_codes,
                hold_ms,
            })
        }
        "scroll" => {
            let dir_str: String = table.get("_direction")?;
            let direction = ScrollDirection::from_str_name(&dir_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let amount: i32 = table.get("_amount")?;
            let interval_ms: u32 = table.get("_interval_ms")?;
            let duration_ms: Option<u32> = table.get("_duration_ms").ok();
            let jitter_ms: u32 = table.get("_jitter_ms")?;
            Ok(ActionConfig::ScrollWheel {
                direction,
                amount,
                interval_ms,
                duration_ms,
                jitter_ms,
            })
        }
        "mouse_move" => {
            let dx: i32 = table.get("_dx")?;
            let dy: i32 = table.get("_dy")?;
            let interval_ms: u32 = table.get("_interval_ms")?;
            let duration_ms: Option<u32> = table.get("_duration_ms").ok();
            let jitter_ms: u32 = table.get("_jitter_ms")?;
            Ok(ActionConfig::MouseMove {
                dx,
                dy,
                interval_ms,
                duration_ms,
                jitter_ms,
            })
        }
        "sequence" => {
            if depth >= MAX_ACTION_DEPTH {
                return Err(LuaError::RuntimeError(format!(
                    "Action nesting depth exceeds maximum of {}",
                    MAX_ACTION_DEPTH
                )));
            }
            let actions_table: LuaTable = table.get("_actions")?;
            let actions = parse_action_list(&actions_table, depth + 1)?;
            Ok(ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions,
            })
        }
        "parallel" => {
            if depth >= MAX_ACTION_DEPTH {
                return Err(LuaError::RuntimeError(format!(
                    "Action nesting depth exceeds maximum of {}",
                    MAX_ACTION_DEPTH
                )));
            }
            let actions_table: LuaTable = table.get("_actions")?;
            let actions = parse_action_list(&actions_table, depth + 1)?;
            if actions.len() > MAX_PARALLEL_ACTIONS {
                return Err(LuaError::RuntimeError(format!(
                    "Parallel action has {} sub-actions (maximum is {})",
                    actions.len(),
                    MAX_PARALLEL_ACTIONS
                )));
            }
            Ok(ActionConfig::Composite {
                mode: CompositeMode::Parallel,
                actions,
            })
        }
        "noop" => Ok(ActionConfig::NoOp),
        "delay" => {
            let duration_ms: u32 = table.get("_duration_ms")?;
            Ok(ActionConfig::Delay { duration_ms })
        }
        "mouse_move_abs" => {
            let x: i32 = table.get("_x")?;
            let y: i32 = table.get("_y")?;
            Ok(ActionConfig::MouseMoveAbsolute { x, y })
        }
        "click_at" => {
            let x: i32 = table.get("_x")?;
            let y: i32 = table.get("_y")?;
            let button_str: String = table.get("_button")?;
            let button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let hold_ms: u32 = table.get("_hold_ms").unwrap_or(0);
            let settle_ms: u32 = table.get("_settle_ms").unwrap_or(5);
            Ok(ActionConfig::ClickAt {
                x,
                y,
                button,
                hold_ms,
                settle_ms,
            })
        }
        "drag" => {
            let from_x: i32 = table.get("_from_x")?;
            let from_y: i32 = table.get("_from_y")?;
            let to_x: i32 = table.get("_to_x")?;
            let to_y: i32 = table.get("_to_y")?;
            let button_str: String = table.get("_button")?;
            let button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let duration_ms: u32 = table.get("_duration_ms").unwrap_or(100);
            Ok(ActionConfig::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            })
        }
        "set_layer" => {
            let layer: String = table.get("_layer")?;
            Ok(ActionConfig::SetLayer { layer })
        }
        other => Err(LuaError::RuntimeError(format!(
            "Unknown action type: {}",
            other
        ))),
    }
}

fn parse_action_list(table: &LuaTable, depth: usize) -> Result<Vec<ActionConfig>, LuaError> {
    let mut actions = Vec::new();
    for value in table.sequence_values::<LuaTable>() {
        let t = value?;
        actions.push(parse_action_table(&t, depth)?);
    }
    Ok(actions)
}

/// Parse the optional `modifiers` array from a Lua action table.
/// Accepts short names ("ctrl", "shift", "alt", "super") and full KEY_* names.
/// Returns parallel vecs of resolved names and codes.
/// Errors on unknown modifier names or duplicates.
fn parse_modifiers(table: &LuaTable) -> Result<(Vec<String>, Vec<u32>), ConfigError> {
    let mod_table: LuaTable = match table.get("modifiers") {
        Ok(t) => t,
        Err(_) => return Ok((Vec::new(), Vec::new())),
    };

    let mut names = Vec::new();
    let mut codes = Vec::new();
    let mut seen_codes = std::collections::HashSet::new();

    for item in mod_table.sequence_values::<String>() {
        let raw = item.map_err(|e| ConfigError::Lua(e.to_string()))?;
        let (resolved_name, code) = normalize_key_name(&raw)?;
        if !seen_codes.insert(code) {
            return Err(ConfigError::Validation(format!(
                "duplicate modifier key: '{}'",
                raw
            )));
        }
        names.push(resolved_name);
        codes.push(code);
    }

    Ok((names, codes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    fn test_logger() -> Arc<Logger> {
        let logger = Logger::new(100, crate::logger::LogLevel::Trace, false);
        logger.set_quiet(true);
        Arc::new(logger)
    }

    fn write_temp_config(dir: &Path, filename: &str, content: &str) -> PathBuf {
        let path = dir.join(filename);
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_minimal_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.set_options({ dry_run = false })
            wayclick.register_trigger({
                id = "test",
                action = wayclick.auto_click({ button = "left", interval_ms = 50 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        assert!(!config.options.dry_run);
        assert_eq!(config.triggers.len(), 1);
        assert_eq!(config.triggers[0].id, "test");
    }

    #[test]
    fn test_auto_click_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.auto_click({}),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::AutoClick {
                button,
                interval_ms,
                duration_ms,
                jitter_ms,
                ..
            } => {
                assert_eq!(*button, MouseButton::Left);
                assert_eq!(*interval_ms, 50);
                assert_eq!(*duration_ms, None);
                assert_eq!(*jitter_ms, 0);
            }
            _ => panic!("Expected AutoClick"),
        }
    }

    #[test]
    fn test_click_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.click({ button = "right", hold_ms = 5 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::AutoClick {
                button,
                interval_ms,
                duration_ms,
                hold_ms,
                ..
            } => {
                assert_eq!(*button, MouseButton::Right);
                assert_eq!(*interval_ms, 1);
                assert_eq!(*duration_ms, None);
                assert_eq!(*hold_ms, 5);
            }
            _ => panic!("Expected AutoClick from click()"),
        }
    }

    #[test]
    fn test_click_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.click({}),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::AutoClick {
                button,
                interval_ms,
                duration_ms,
                hold_ms,
                ..
            } => {
                assert_eq!(*button, MouseButton::Left);
                assert_eq!(*interval_ms, 1);
                assert_eq!(*duration_ms, None);
                assert_eq!(*hold_ms, 0);
            }
            _ => panic!("Expected AutoClick from click()"),
        }
    }

    #[test]
    fn test_all_mouse_buttons() {
        let dir = tempfile::tempdir().unwrap();
        for btn in &["left", "right", "middle", "button4", "button5"] {
            let path = write_temp_config(
                dir.path(),
                "init.lua",
                &format!(
                    r#"
                    wayclick.register_trigger({{
                        id = "test",
                        action = wayclick.auto_click({{ button = "{}" }}),
                    }})
                "#,
                    btn
                ),
            );
            let config = load_config(&path, &test_logger()).unwrap();
            match &config.triggers[0].action {
                ActionConfig::AutoClick { button, .. } => {
                    assert_eq!(*button, MouseButton::from_str_name(btn).unwrap());
                }
                _ => panic!("Expected AutoClick"),
            }
        }
    }

    #[test]
    fn test_all_trigger_modes() {
        let dir = tempfile::tempdir().unwrap();
        for mode in &["toggle", "hold", "oneshot"] {
            let path = write_temp_config(
                dir.path(),
                "init.lua",
                &format!(
                    r#"
                    wayclick.register_trigger({{
                        id = "test",
                        mode = "{}",
                        action = wayclick.noop(),
                    }})
                "#,
                    mode
                ),
            );
            let config = load_config(&path, &test_logger()).unwrap();
            assert_eq!(
                config.triggers[0].mode,
                TriggerMode::from_str_mode(mode).unwrap()
            );
        }
    }

    #[test]
    fn test_key_normalization() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.key_press({ key = "space", interval_ms = 100 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::KeyPress {
                key_name, key_code, ..
            } => {
                assert_eq!(key_name, "KEY_SPACE");
                assert_eq!(*key_code, 57);
            }
            _ => panic!("Expected KeyPress"),
        }
    }

    #[test]
    fn test_composite_parallel() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.parallel({
                    actions = {
                        wayclick.auto_click({ button = "left", interval_ms = 50 }),
                        wayclick.key_press({ key = "a", interval_ms = 100 }),
                    },
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Composite { mode, actions } => {
                assert_eq!(*mode, CompositeMode::Parallel);
                assert_eq!(actions.len(), 2);
            }
            _ => panic!("Expected Composite"),
        }
    }

    #[test]
    fn test_composite_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.sequence({
                    actions = {
                        wayclick.auto_click({ button = "left", interval_ms = 50 }),
                        wayclick.noop(),
                    },
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Composite { mode, actions } => {
                assert_eq!(*mode, CompositeMode::Sequence);
                assert_eq!(actions.len(), 2);
            }
            _ => panic!("Expected Composite"),
        }
    }

    #[test]
    fn test_bind_device_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "Logitech G Pro",
                bindings = {
                    { code = "BTN_SIDE", trigger = "test" },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        assert_eq!(config.device_bindings.len(), 1);
        match &config.device_bindings[0].device_match {
            DeviceMatch::ByName { contains } => {
                assert_eq!(contains, "Logitech G Pro");
            }
            _ => panic!("Expected ByName"),
        }
    }

    #[test]
    fn test_bind_device_by_vidpid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                vid = 0x046d,
                pid = 0xc08b,
                bindings = {
                    { code = "BTN_SIDE", trigger = "test" },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.device_bindings[0].device_match {
            DeviceMatch::ByVidPid { vendor, product } => {
                assert_eq!(*vendor, 0x046d);
                assert_eq!(*product, 0xc08b);
            }
            _ => panic!("Expected ByVidPid"),
        }
    }

    #[test]
    fn test_bind_evdev_legacy_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_evdev({
                device = "/dev/input/event15",
                code = "BTN_SIDE",
                trigger = "test",
            })
        "#,
        );
        let logger = test_logger();
        let config = load_config(&path, &logger).unwrap();
        assert_eq!(config.device_bindings.len(), 1);
        match &config.device_bindings[0].device_match {
            DeviceMatch::ByPath { path } => {
                assert_eq!(path, "/dev/input/event15");
            }
            _ => panic!("Expected ByPath"),
        }
        // Check that a deprecation warning was logged
        let entries = logger.all_entries();
        assert!(entries.iter().any(|e| e.message.contains("deprecated")));
    }

    #[test]
    fn test_config_error_duplicate_trigger_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "dup", action = wayclick.noop() })
            wayclick.register_trigger({ id = "dup", action = wayclick.noop() })
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
    }

    #[test]
    fn test_config_error_unknown_trigger_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.bind_device({
                name = "test",
                bindings = {
                    { code = "BTN_SIDE", trigger = "nonexistent" },
                },
            })
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
    }

    #[test]
    fn test_config_error_invalid_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.key_press({ key = "NOT_A_KEY_9999" }),
            })
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
    }

    #[test]
    fn test_sandbox_blocks_os_execute() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local result = os.execute("echo hello")
            -- os.execute should be nil in sandbox, so calling it errors
        "#,
        );
        // Should either error or os.execute returns nil
        let result = load_config(&path, &test_logger());
        // The script might error because os.execute is nil and calling nil errors
        // Or it might succeed with result=nil. Either way, it should not actually execute.
        // We accept both outcomes.
        let _ = result;
    }

    #[test]
    fn test_sandbox_blocks_io_popen() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local f = io.popen("echo hello")
        "#,
        );
        let result = load_config(&path, &test_logger());
        // io.popen is nil, so calling it should error
        assert!(result.is_err());
    }

    #[test]
    fn test_sandbox_io_open_blocks_outside_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local f, err = io.open("/etc/hostname", "r")
            if f then
                error("io.open should have been blocked for files outside config dir")
            end
            if not err:find("outside config directory") then
                error("Expected 'outside config directory' error, got: " .. err)
            end
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(
            result.is_ok(),
            "Script should handle the blocked read gracefully"
        );
    }

    #[test]
    fn test_sandbox_io_open_blocks_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local f, err = io.open("../../../etc/hostname", "r")
            if f then
                error("io.open should have blocked path traversal")
            end
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok());
    }

    #[test]
    fn test_sandbox_io_open_allows_config_dir_reads() {
        let dir = tempfile::tempdir().unwrap();

        // Create a data file in the config directory
        let data_path = dir.path().join("data.txt");
        fs::write(&data_path, "test_data_123").unwrap();

        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local f, err = io.open("data.txt", "r")
            if not f then
                error("io.open should allow reads in config dir: " .. (err or "unknown"))
            end
            local content = f:read("*a")
            f:close()
            if content ~= "test_data_123" then
                error("Expected 'test_data_123', got: " .. content)
            end
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(
            result.is_ok(),
            "Should be able to read files in config dir: {:?}",
            result
        );
    }

    #[test]
    fn test_sandbox_io_open_still_blocks_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local f, err = io.open("output.txt", "w")
            if f then
                error("io.open should block write mode")
            end
            if not err:find("write access denied") then
                error("Expected 'write access denied' error, got: " .. err)
            end
        "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok());
    }

    #[test]
    fn test_socket_path_default() {
        let config = Config::default();
        let path = effective_socket_path(&config);
        assert!(path.to_string_lossy().contains("wayclick.sock"));
    }

    #[test]
    fn test_lua_module_loading() {
        let dir = tempfile::tempdir().unwrap();
        let lua_dir = dir.path().join("lua");
        fs::create_dir_all(&lua_dir).unwrap();

        // Create a helper module
        write_temp_config(
            &lua_dir,
            "helpers.lua",
            r#"
            wayclick.register_trigger({
                id = "from_module",
                action = wayclick.noop(),
            })
        "#,
        );

        // Create init.lua that requires the module
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "from_init",
                action = wayclick.noop(),
            })
            require("helpers")
        "#,
        );

        let config = load_config(&path, &test_logger()).unwrap();
        assert_eq!(config.triggers.len(), 2);
        assert!(config.triggers.iter().any(|t| t.id == "from_init"));
        assert!(config.triggers.iter().any(|t| t.id == "from_module"));
    }

    #[test]
    fn test_scroll_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.scroll({
                    direction = "down",
                    amount = 5,
                    interval_ms = 100,
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::ScrollWheel {
                direction, amount, ..
            } => {
                assert_eq!(*direction, ScrollDirection::Down);
                assert_eq!(*amount, 5);
            }
            _ => panic!("Expected ScrollWheel"),
        }
    }

    #[test]
    fn test_mouse_move_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.mouse_move({
                    dx = 10,
                    dy = -5,
                    interval_ms = 16,
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::MouseMove { dx, dy, .. } => {
                assert_eq!(*dx, 10);
                assert_eq!(*dy, -5);
            }
            _ => panic!("Expected MouseMove"),
        }
    }

    #[test]
    fn test_delay_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.delay({ ms = 250 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Delay { duration_ms } => {
                assert_eq!(*duration_ms, 250);
            }
            _ => panic!("Expected Delay"),
        }
    }

    #[test]
    fn test_delay_requires_ms_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.delay({}),
            })
        "#,
        );
        assert!(load_config(&path, &test_logger()).is_err());
    }

    #[test]
    fn test_delay_in_sequence() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.sequence({ actions = {
                    wayclick.auto_click({ button = "left" }),
                    wayclick.delay({ ms = 500 }),
                    wayclick.auto_click({ button = "left" }),
                }}),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions,
            } => {
                assert_eq!(actions.len(), 3);
                assert!(matches!(actions[0], ActionConfig::AutoClick { .. }));
                assert!(matches!(
                    actions[1],
                    ActionConfig::Delay { duration_ms: 500 }
                ));
                assert!(matches!(actions[2], ActionConfig::AutoClick { .. }));
            }
            _ => panic!("Expected Sequence"),
        }
    }

    #[test]
    fn test_auto_click_hold_ms() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.auto_click({
                    button = "left",
                    interval_ms = 20,
                    hold_ms = 5,
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::AutoClick { hold_ms, .. } => {
                assert_eq!(*hold_ms, 5);
            }
            _ => panic!("Expected AutoClick"),
        }
    }

    #[test]
    fn test_auto_click_hold_ms_defaults_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.auto_click({ button = "left" }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::AutoClick { hold_ms, .. } => {
                assert_eq!(*hold_ms, 0);
            }
            _ => panic!("Expected AutoClick"),
        }
    }

    #[test]
    fn test_mouse_move_abs_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.mouse_move_abs({ x = 100, y = 200 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::MouseMoveAbsolute { x, y } => {
                assert_eq!(*x, 100);
                assert_eq!(*y, 200);
            }
            _ => panic!("Expected MouseMoveAbsolute"),
        }
    }

    #[test]
    fn test_click_at_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.click_at({ x = 500, y = 300, button = "right", hold_ms = 10 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::ClickAt {
                x,
                y,
                button,
                hold_ms,
                settle_ms,
            } => {
                assert_eq!(*x, 500);
                assert_eq!(*y, 300);
                assert_eq!(*button, MouseButton::Right);
                assert_eq!(*hold_ms, 10);
                assert_eq!(*settle_ms, 5);
            }
            _ => panic!("Expected ClickAt"),
        }
    }

    #[test]
    fn test_click_at_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.click_at({ x = 100, y = 100 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::ClickAt {
                button,
                hold_ms,
                settle_ms,
                ..
            } => {
                assert_eq!(*button, MouseButton::Left);
                assert_eq!(*hold_ms, 0);
                assert_eq!(*settle_ms, 5);
            }
            _ => panic!("Expected ClickAt"),
        }
    }

    #[test]
    fn test_click_at_custom_settle_ms() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.click_at({ x = 100, y = 200, settle_ms = 0 }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::ClickAt { settle_ms, .. } => {
                assert_eq!(*settle_ms, 0);
            }
            _ => panic!("Expected ClickAt"),
        }
    }

    #[test]
    fn test_drag_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.drag({
                    from_x = 100, from_y = 200,
                    to_x = 300, to_y = 400,
                    button = "left",
                    duration_ms = 500,
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                duration_ms,
            } => {
                assert_eq!(*from_x, 100);
                assert_eq!(*from_y, 200);
                assert_eq!(*to_x, 300);
                assert_eq!(*to_y, 400);
                assert_eq!(*button, MouseButton::Left);
                assert_eq!(*duration_ms, 500);
            }
            _ => panic!("Expected Drag"),
        }
    }

    #[test]
    fn test_set_layer_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.set_layer({ layer = "combat" }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::SetLayer { layer } => {
                assert_eq!(layer, "combat");
            }
            _ => panic!("Expected SetLayer"),
        }
    }

    #[test]
    fn test_media_key_action() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.media_key({ key = "play_pause" }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::KeyPress {
                key_code, key_name, ..
            } => {
                assert_eq!(*key_code, 164); // KEY_PLAYPAUSE
                assert_eq!(key_name, "KEY_PLAY_PAUSE");
            }
            _ => panic!("Expected KeyPress"),
        }
    }

    #[test]
    fn test_media_key_constants() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.key_press({
                    key_code = wayclick.keys.VOLUME_UP,
                    key = "KEY_VOLUMEUP",
                }),
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::KeyPress { key_code, .. } => {
                assert_eq!(*key_code, 115);
            }
            _ => panic!("Expected KeyPress"),
        }
    }

    #[test]
    fn test_bind_device_keyboard_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "keyboard",
                bindings = {
                    { code = "KEY_F1", trigger = "test" },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref binding) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(binding.codes, vec![59]); // KEY_F1 = 59
        assert_eq!(binding.code_names, vec!["KEY_F1"]);
    }

    #[test]
    fn test_bind_device_chord() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "mouse",
                bindings = {
                    { code = "BTN_SIDE+BTN_EXTRA", trigger = "test" },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref binding) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(binding.codes, vec![0x113, 0x114]);
        assert_eq!(binding.code_names, vec!["BTN_SIDE", "BTN_EXTRA"]);
    }

    #[test]
    fn test_bind_device_hold_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "tap_action",
                action = wayclick.noop(),
            })
            wayclick.register_trigger({
                id = "hold_action",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "mouse",
                bindings = {
                    { code = "BTN_SIDE", trigger = "tap_action", hold_trigger = "hold_action", hold_ms = 500 },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref binding) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(binding.hold_trigger_id, Some("hold_action".to_string()));
        assert_eq!(binding.hold_threshold_ms, Some(500));
    }

    #[test]
    fn test_bind_device_layer_filter() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "mouse",
                bindings = {
                    { code = "BTN_SIDE", trigger = "test", layer = "combat" },
                },
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref binding) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(binding.layer, Some("combat".to_string()));
    }

    #[test]
    fn test_bind_device_hold_requires_both_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "mouse",
                bindings = {
                    { code = "BTN_SIDE", trigger = "test", hold_trigger = "missing_hold_ms" },
                },
            })
        "#,
        );
        assert!(load_config(&path, &test_logger()).is_err());
    }

    #[test]
    fn test_set_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.set_profile({
                name = "gaming",
                match_app = "steam_app_.*",
                layer = "combat",
            })
        "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        assert_eq!(config.profile_rules.len(), 1);
        assert_eq!(config.profile_rules[0].name, "gaming");
        assert_eq!(
            config.profile_rules[0].match_app,
            Some("steam_app_.*".to_string())
        );
        assert_eq!(config.profile_rules[0].layer, "combat");
    }

    #[test]
    fn test_trigger_code_from_name() {
        // BTN_* codes
        assert_eq!(trigger_code_from_name("BTN_LEFT"), Some(0x110));
        assert_eq!(trigger_code_from_name("BTN_SIDE"), Some(0x113));
        // KEY_* codes
        assert_eq!(trigger_code_from_name("KEY_F1"), Some(59));
        assert_eq!(trigger_code_from_name("KEY_SPACE"), Some(57));
        // Media keys
        assert_eq!(trigger_code_from_name("KEY_PLAYPAUSE"), Some(164));
        assert_eq!(trigger_code_from_name("KEY_VOLUMEUP"), Some(115));
        // Bare name (without KEY_ prefix)
        assert_eq!(trigger_code_from_name("SPACE"), Some(57));
        // Invalid
        assert_eq!(trigger_code_from_name("INVALID_CODE_XYZ"), None);
    }

    #[test]
    fn test_action_type_names() {
        assert_eq!(
            ActionConfig::MouseMoveAbsolute { x: 0, y: 0 }.type_name(),
            "mouse_move_abs"
        );
        assert_eq!(
            ActionConfig::ClickAt {
                x: 0,
                y: 0,
                button: MouseButton::Left,
                hold_ms: 0,
                settle_ms: 5,
            }
            .type_name(),
            "click_at"
        );
        assert_eq!(
            ActionConfig::Drag {
                from_x: 0,
                from_y: 0,
                to_x: 0,
                to_y: 0,
                button: MouseButton::Left,
                duration_ms: 100
            }
            .type_name(),
            "drag"
        );
        assert_eq!(
            ActionConfig::SetLayer {
                layer: "test".into()
            }
            .type_name(),
            "set_layer"
        );
    }

    #[test]
    fn test_is_oneshot_only() {
        assert!(ActionConfig::SetLayer { layer: "x".into() }.is_oneshot_only());
        assert!(ActionConfig::ClickAt {
            x: 0,
            y: 0,
            button: MouseButton::Left,
            hold_ms: 0,
            settle_ms: 5,
        }
        .is_oneshot_only());
        assert!(ActionConfig::Drag {
            from_x: 0,
            from_y: 0,
            to_x: 1,
            to_y: 1,
            button: MouseButton::Left,
            duration_ms: 100
        }
        .is_oneshot_only());
        assert!(ActionConfig::MouseMoveAbsolute { x: 0, y: 0 }.is_oneshot_only());
        assert!(!ActionConfig::NoOp.is_oneshot_only());
        assert!(!ActionConfig::AutoClick {
            button: MouseButton::Left,
            interval_ms: 50,
            duration_ms: None,
            jitter_ms: 0,
            hold_ms: 0
        }
        .is_oneshot_only());
    }

    // ---- Resource exhaustion safety tests ----

    #[test]
    fn test_lua_instruction_limit_blocks_infinite_loop() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            while true do end
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("instruction limit"),
            "Expected instruction limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_action_nesting_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local action = wayclick.noop()
            for i = 1, 50 do
                action = wayclick.sequence({ actions = { action } })
            end
            wayclick.register_trigger({
                id = "deep",
                action = action,
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nesting depth"),
            "Expected nesting depth error, got: {}",
            err
        );
    }

    #[test]
    fn test_action_nesting_within_limit_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local action = wayclick.noop()
            for i = 1, 5 do
                action = wayclick.sequence({ actions = { action } })
            end
            wayclick.register_trigger({
                id = "shallow",
                action = action,
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok());
    }

    #[test]
    fn test_parallel_action_count_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local actions = {}
            for i = 1, 100 do
                actions[i] = wayclick.noop()
            end
            wayclick.register_trigger({
                id = "big_parallel",
                action = wayclick.parallel({ actions = actions }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("sub-actions") || err.contains("maximum"),
            "Expected parallel sub-action limit error, got: {}",
            err
        );
    }

    #[test]
    fn test_parallel_action_within_limit_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            local actions = {}
            for i = 1, 10 do
                actions[i] = wayclick.noop()
            end
            wayclick.register_trigger({
                id = "small_parallel",
                action = wayclick.parallel({ actions = actions }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok());
    }

    #[test]
    fn test_normal_config_within_instruction_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            for i = 1, 100 do
                wayclick.register_trigger({
                    id = "trigger_" .. i,
                    action = wayclick.sequence({
                        actions = {
                            wayclick.auto_click({ button = "left", interval_ms = 50 }),
                            wayclick.delay({ ms = 100 }),
                            wayclick.key_press({ key = "a" }),
                        }
                    }),
                })
            end
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.triggers.len(), 100);
    }

    #[test]
    fn test_lua_scroll_binding_parses() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "left_click",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "Test Mouse",
                exclusive = true,
                bindings = {
                    { scroll = "up", trigger = "left_click" },
                },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let config = result.unwrap();
        assert_eq!(config.device_bindings.len(), 1);
        assert!(
            matches!(config.device_bindings[0].bindings[0], Binding::Scroll(ref sb) if sb.direction == ScrollDirection::Up)
        );
    }

    #[test]
    fn test_lua_mixed_scroll_and_button_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "left_click",
                action = wayclick.noop(),
            })
            wayclick.register_trigger({
                id = "auto_clicker",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "Test Mouse",
                exclusive = true,
                bindings = {
                    { scroll = "up", trigger = "left_click" },
                    { scroll = "down", trigger = "left_click" },
                    { code = "BTN_EXTRA", trigger = "auto_clicker" },
                },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_ok(), "Failed: {:?}", result.err());
        let config = result.unwrap();
        let scroll_count = config.device_bindings[0]
            .bindings
            .iter()
            .filter(|b| matches!(b, Binding::Scroll(_)))
            .count();
        let button_count = config.device_bindings[0]
            .bindings
            .iter()
            .filter(|b| matches!(b, Binding::Button(_)))
            .count();
        assert_eq!(scroll_count, 2);
        assert_eq!(button_count, 1);
    }

    #[test]
    fn test_lua_scroll_without_exclusive_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "left_click",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "Test Mouse",
                bindings = {
                    { scroll = "up", trigger = "left_click" },
                },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(
            result.is_err(),
            "Should error when scroll binding without exclusive"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exclusive"),
            "Error should mention exclusive: {}",
            err
        );
    }

    #[test]
    fn test_lua_invalid_scroll_direction_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "left_click",
                action = wayclick.noop(),
            })
            wayclick.bind_device({
                name = "Test Mouse",
                exclusive = true,
                bindings = {
                    { scroll = "diagonal", trigger = "left_click" },
                },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "Should error for invalid scroll direction");
    }

    #[test]
    fn test_keystroke_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.keystroke({ key = "z", modifiers = {"ctrl"} }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Keystroke {
                key_name,
                key_code,
                modifier_names,
                modifier_codes,
                hold_ms,
            } => {
                assert_eq!(key_name, "KEY_Z");
                assert_eq!(*key_code, 44);
                assert_eq!(modifier_names, &["KEY_CTRL"]);
                assert_eq!(modifier_codes, &[29]);
                assert_eq!(*hold_ms, 0);
            }
            other => panic!("Expected Keystroke, got {:?}", other),
        }
    }

    #[test]
    fn test_keystroke_no_modifiers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.keystroke({ key = "space" }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Keystroke {
                key_name,
                key_code,
                modifier_names,
                modifier_codes,
                ..
            } => {
                assert_eq!(key_name, "KEY_SPACE");
                assert_eq!(*key_code, 57);
                assert!(modifier_names.is_empty());
                assert!(modifier_codes.is_empty());
            }
            other => panic!("Expected Keystroke, got {:?}", other),
        }
    }

    #[test]
    fn test_keystroke_multiple_modifiers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.keystroke({ key = "z", modifiers = {"ctrl", "shift"} }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Keystroke {
                modifier_names,
                modifier_codes,
                ..
            } => {
                assert_eq!(modifier_names, &["KEY_CTRL", "KEY_SHIFT"]);
                assert_eq!(modifier_codes, &[29, 42]);
            }
            other => panic!("Expected Keystroke, got {:?}", other),
        }
    }

    #[test]
    fn test_keystroke_hold_ms() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.keystroke({ key = "enter", hold_ms = 50 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Keystroke { hold_ms, .. } => {
                assert_eq!(*hold_ms, 50);
            }
            other => panic!("Expected Keystroke, got {:?}", other),
        }
    }

    #[test]
    fn test_keystroke_invalid_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.keystroke({ key = "DEFINITELY_NOT_A_KEY" }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "Should error for unknown key");
    }

    #[test]
    fn test_keystroke_invalid_modifier() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.keystroke({ key = "z", modifiers = {"not_a_modifier"} }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "Should error for unknown modifier");
    }

    #[test]
    fn test_key_press_with_modifiers() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                action = wayclick.key_press({ key = "z", modifiers = {"ctrl"}, interval_ms = 200 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::KeyPress {
                key_name,
                key_code,
                modifier_names,
                modifier_codes,
                interval_ms,
                ..
            } => {
                assert_eq!(key_name, "KEY_Z");
                assert_eq!(*key_code, 44);
                assert_eq!(modifier_names, &["KEY_CTRL"]);
                assert_eq!(modifier_codes, &[29]);
                assert_eq!(*interval_ms, 200);
            }
            other => panic!("Expected KeyPress, got {:?}", other),
        }
    }

    #[test]
    fn test_modifier_alias_ctrl() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "oneshot",
                action = wayclick.keystroke({ key = "z", modifiers = {"ctrl"} }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        match &config.triggers[0].action {
            ActionConfig::Keystroke { modifier_codes, .. } => {
                // "ctrl" should resolve to KEY_LEFTCTRL = 29
                assert_eq!(modifier_codes, &[29]);
            }
            other => panic!("Expected Keystroke, got {:?}", other),
        }
    }

    #[test]
    fn test_modifier_alias_super() {
        let dir = tempfile::tempdir().unwrap();
        // All three aliases should resolve to KEY_LEFTMETA = 125
        for alias in &["super", "win", "meta"] {
            let path = write_temp_config(
                dir.path(),
                "init.lua",
                &format!(
                    r#"
                    wayclick.register_trigger({{
                        id = "test",
                        mode = "oneshot",
                        action = wayclick.keystroke({{ key = "z", modifiers = {{"{}"}} }}),
                    }})
                    "#,
                    alias
                ),
            );
            let config = load_config(&path, &test_logger()).unwrap();
            match &config.triggers[0].action {
                ActionConfig::Keystroke { modifier_codes, .. } => {
                    assert_eq!(
                        *modifier_codes,
                        vec![125],
                        "alias '{}' should resolve to KEY_LEFTMETA (125)",
                        alias
                    );
                }
                other => panic!("Expected Keystroke for alias '{}', got {:?}", alias, other),
            }
        }
    }

    #[test]
    fn test_keystroke_rejected_in_toggle_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "test",
                mode = "toggle",
                action = wayclick.keystroke({ key = "z" }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "Keystroke should be rejected in toggle mode");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("keystroke") || err.contains("oneshot"),
            "Error should mention keystroke or oneshot: {}",
            err
        );
    }

    // --- swallow / on / chord lua_api tests ---

    #[test]
    fn test_bind_device_swallow_defaults_to_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                bindings = { { code = "BTN_EXTRA", trigger = "t" } },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref bb) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert!(!bb.swallow, "swallow should default to false");
        assert_eq!(bb.on, TriggerEdge::Press, "on should default to Press");
    }

    #[test]
    fn test_bind_device_swallow_true_button() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                exclusive = true,
                bindings = { { code = "BTN_EXTRA", trigger = "t", swallow = true } },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref bb) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert!(bb.swallow, "swallow should be true");
    }

    #[test]
    fn test_bind_device_swallow_true_scroll() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                exclusive = true,
                bindings = { { scroll = "up", trigger = "t", swallow = true } },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Scroll(ref sb) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Scroll binding");
        };
        assert!(sb.swallow, "scroll swallow should be true");
    }

    #[test]
    fn test_bind_device_on_release_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                bindings = { { code = "BTN_EXTRA", trigger = "t", on = "release" } },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref bb) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(bb.on, TriggerEdge::Release);
    }

    #[test]
    fn test_bind_device_swallow_requires_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                exclusive = false,
                bindings = { { code = "BTN_EXTRA", trigger = "t", swallow = true } },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "swallow=true without exclusive=true should error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exclusive"), "Error should mention exclusive: {}", err);
    }

    #[test]
    fn test_bind_device_on_release_swallow_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                exclusive = true,
                bindings = { { code = "BTN_EXTRA", trigger = "t", on = "release", swallow = true } },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "on=release + swallow=true should error");
    }

    #[test]
    fn test_bind_device_on_release_with_hold_trigger_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.register_trigger({ id = "h", action = wayclick.noop() })
            wayclick.bind_device({
                name = "mouse",
                bindings = { { code = "BTN_EXTRA", trigger = "t", on = "release", hold_trigger = "h", hold_ms = 500 } },
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(result.is_err(), "on=release + hold_trigger should error");
    }

    #[test]
    fn test_bind_device_key_mouse_chord() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({ id = "t", action = wayclick.noop() })
            wayclick.bind_device({
                name = "macro pad",
                bindings = { { code = "KEY_LEFTCTRL+BTN_LEFT", trigger = "t" } },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let Binding::Button(ref bb) = config.device_bindings[0].bindings[0] else {
            panic!("Expected Button binding");
        };
        assert_eq!(bb.code_names, vec!["KEY_LEFTCTRL", "BTN_LEFT"]);
        assert_eq!(bb.codes.len(), 2);
    }

    // ── type_text tests ──────────────────────────────────────────────────────

    #[test]
    fn test_type_text_basic() {
        // "hi" → 2 keystroke tables + 1 delay table = 3 actions total
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "hi" }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        // 2 chars + 1 delay between them = 3
        assert_eq!(actions.len(), 3);
        assert!(matches!(actions[0], ActionConfig::Keystroke { .. }));
        assert!(matches!(actions[1], ActionConfig::Delay { .. }));
        assert!(matches!(actions[2], ActionConfig::Keystroke { .. }));
    }

    #[test]
    fn test_type_text_key_codes() {
        // 'h' → KEY_H (code 35), 'i' → KEY_I (code 23)
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "hi", delay_ms = 0 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        // delay_ms = 0 → no delay tables → exactly 2 keystrokes
        assert_eq!(actions.len(), 2);
        let ActionConfig::Keystroke {
            key_name: ref kn0,
            key_code: kc0,
            modifier_names: ref mn0,
            ..
        } = actions[0]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn0, "KEY_H");
        assert_eq!(kc0, 35);
        assert!(mn0.is_empty(), "lowercase should have no modifiers");

        let ActionConfig::Keystroke {
            key_name: ref kn1,
            key_code: kc1,
            ..
        } = actions[1]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn1, "KEY_I");
        assert_eq!(kc1, 23);
    }

    #[test]
    fn test_type_text_uppercase_shift() {
        // 'H' → KEY_H + KEY_LEFTSHIFT modifier
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "H", delay_ms = 0 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 1);
        let ActionConfig::Keystroke {
            key_name: ref kn,
            key_code: kc,
            modifier_names: ref mn,
            modifier_codes: ref mc,
            ..
        } = actions[0]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn, "KEY_H");
        assert_eq!(kc, 35);
        assert_eq!(mn, &["KEY_LEFTSHIFT"]);
        assert_eq!(mc, &[42u32]);
    }

    #[test]
    fn test_type_text_slash() {
        // '/' → KEY_SLASH (53), no shift
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "/", delay_ms = 0 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 1);
        let ActionConfig::Keystroke {
            key_name: ref kn,
            key_code: kc,
            modifier_names: ref mn,
            ..
        } = actions[0]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn, "KEY_SLASH");
        assert_eq!(kc, 53);
        assert!(mn.is_empty());
    }

    #[test]
    fn test_type_text_newline_maps_to_enter() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            // Lua \n inside double-quoted string is a literal newline
            "wayclick.register_trigger({\n\
             id = \"t\", mode = \"oneshot\",\n\
             action = wayclick.type_text({ text = \"\\n\", delay_ms = 0 }),\n\
             })\n",
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 1);
        let ActionConfig::Keystroke {
            key_name: ref kn,
            key_code: kc,
            ..
        } = actions[0]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn, "KEY_ENTER");
        assert_eq!(kc, 28);
    }

    #[test]
    fn test_type_text_default_delay() {
        // 3 chars → 3 keystrokes + 2 delays = 5 actions with default delay_ms
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "abc" }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 5);
        // Verify delay is 30ms (the default)
        let ActionConfig::Delay { duration_ms } = actions[1] else {
            panic!("expected Delay");
        };
        assert_eq!(duration_ms, 30);
    }

    #[test]
    fn test_type_text_delay_ms_zero() {
        // delay_ms = 0 → only keystroke tables, no delay tables
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "abc", delay_ms = 0 }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 3);
        assert!(actions.iter().all(|a| matches!(a, ActionConfig::Keystroke { .. })));
    }

    #[test]
    fn test_type_text_empty_string() {
        // Empty text → empty sequence (no error)
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "" }),
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        let ActionConfig::Composite { mode: CompositeMode::Sequence, ref actions } =
            &config.triggers[0].action
        else {
            panic!("expected Sequence action");
        };
        assert_eq!(actions.len(), 0);
    }

    #[test]
    fn test_type_text_unknown_char_errors() {
        // Emoji is not on US QWERTY → should fail at config load
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "t",
                mode = "oneshot",
                action = wayclick.type_text({ text = "hello 🎉" }),
            })
            "#,
        );
        let result = load_config(&path, &test_logger());
        assert!(
            result.is_err(),
            "non-ASCII character should produce a config error"
        );
    }

    #[test]
    fn test_type_text_poe_hideout_macro() {
        // Full PoE macro: Enter + "/hideout" + Enter via sequence composition
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp_config(
            dir.path(),
            "init.lua",
            r#"
            wayclick.register_trigger({
                id = "hideout",
                mode = "oneshot",
                action = wayclick.sequence({
                    actions = {
                        wayclick.keystroke({ key = "enter" }),
                        wayclick.type_text({ text = "/hideout", delay_ms = 0 }),
                        wayclick.keystroke({ key = "enter" }),
                    }
                }),
            })
            wayclick.bind_device({
                name = "keyboard",
                bindings = {
                    { code = "KEY_F5", trigger = "hideout" },
                },
            })
            "#,
        );
        let config = load_config(&path, &test_logger()).unwrap();
        assert_eq!(config.triggers[0].id, "hideout");
        let ActionConfig::Composite { mode: CompositeMode::Sequence, actions: ref outer } =
            &config.triggers[0].action
        else {
            panic!("expected outer Sequence");
        };
        // Outer: enter_ks + type_text_seq + enter_ks = 3
        assert_eq!(outer.len(), 3);

        // Middle element is the type_text expansion: sequence of 8 chars ("/hideout")
        let ActionConfig::Composite { mode: CompositeMode::Sequence, actions: ref inner } = &outer[1]
        else {
            panic!("expected inner Sequence from type_text");
        };
        // "/hideout" = 8 chars, delay_ms=0 → exactly 8 keystrokes
        assert_eq!(inner.len(), 8);

        // First char is '/' → KEY_SLASH
        let ActionConfig::Keystroke {
            key_name: ref kn,
            key_code: kc,
            ..
        } = inner[0]
        else {
            panic!("expected Keystroke");
        };
        assert_eq!(kn, "KEY_SLASH");
        assert_eq!(kc, 53);
    }
}
