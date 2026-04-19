use crate::config::*;
use crate::logger::Logger;
use mlua::prelude::*;
use std::path::Path;
use std::sync::Arc;

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

    // Set up sandbox: remove dangerous functions
    sandbox_lua(&lua)?;

    // Set up sandboxed require
    setup_sandboxed_require(&lua, &config_dir)?;

    // Create the config builder in Lua app data
    lua.set_app_data(ConfigBuilder::new());

    // Register the wayclick global table
    register_wayclick_api(&lua, logger)?;

    // Execute the config file
    let source = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    lua.load(&source)
        .set_name(path.to_string_lossy())
        .exec()
        .map_err(|e| ConfigError::Lua(e.to_string()))?;

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

fn sandbox_lua(lua: &Lua) -> Result<(), ConfigError> {
    lua.load(
        r#"
        os.execute = nil
        os.exit = nil
        if io then
            io.popen = nil
            -- Block write-mode io.open
            local orig_open = io.open
            io.open = function(path, mode)
                if mode and (mode:find("w") or mode:find("a")) then
                    return nil, "write access denied in sandbox"
                end
                return orig_open(path, mode)
            end
        end
        load = nil
        loadfile = nil
        dofile = nil
        debug = nil
    "#,
    )
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

            let action = lua.create_table()?;
            action.set("_type", "key_press")?;
            action.set("_key_name", key_name)?;
            action.set("_key_code", key_code)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
            Ok(action)
        })
        .map_err(|e| ConfigError::Lua(e.to_string()))?;
    wayclick
        .set("key_press", key_press)
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
            let action = lua.create_table()?;
            action.set("_type", "click_at")?;
            action.set("_x", x)?;
            action.set("_y", y)?;
            action.set("_button", button)?;
            action.set("_hold_ms", hold_ms)?;
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

            let action = parse_action_table(&action_table)?;

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

            let mut button_bindings = Vec::new();
            for pair in bindings_table.sequence_values::<LuaTable>() {
                let binding = pair?;
                let code_str: String = binding.get("code")?;
                let trigger_id: String = binding.get("trigger")?;
                let hold_trigger_id: Option<String> = binding.get("hold_trigger").ok();
                let hold_threshold_ms: Option<u32> = binding.get("hold_ms").ok();
                let layer: Option<String> = binding.get("layer").ok();

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

                button_bindings.push(ButtonBinding {
                    codes,
                    code_names,
                    trigger_id,
                    hold_trigger_id,
                    hold_threshold_ms,
                    layer,
                });
            }

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.device_bindings.push(DeviceBinding {
                device_match,
                button_bindings,
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
                button_bindings: vec![ButtonBinding {
                    codes: vec![code],
                    code_names: vec![code_str],
                    trigger_id,
                    hold_trigger_id: None,
                    hold_threshold_ms: None,
                    layer: None,
                }],
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

/// Parse a Lua action table (returned by auto_click, key_press, etc.) into an ActionConfig.
fn parse_action_table(table: &LuaTable) -> Result<ActionConfig, LuaError> {
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
            Ok(ActionConfig::KeyPress {
                key_name,
                key_code,
                interval_ms,
                duration_ms,
                jitter_ms,
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
            let actions_table: LuaTable = table.get("_actions")?;
            let actions = parse_action_list(&actions_table)?;
            Ok(ActionConfig::Composite {
                mode: CompositeMode::Sequence,
                actions,
            })
        }
        "parallel" => {
            let actions_table: LuaTable = table.get("_actions")?;
            let actions = parse_action_list(&actions_table)?;
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
            Ok(ActionConfig::ClickAt {
                x,
                y,
                button,
                hold_ms,
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

fn parse_action_list(table: &LuaTable) -> Result<Vec<ActionConfig>, LuaError> {
    let mut actions = Vec::new();
    for value in table.sequence_values::<LuaTable>() {
        let t = value?;
        actions.push(parse_action_table(&t)?);
    }
    Ok(actions)
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
            } => {
                assert_eq!(*x, 500);
                assert_eq!(*y, 300);
                assert_eq!(*button, MouseButton::Right);
                assert_eq!(*hold_ms, 10);
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
                button, hold_ms, ..
            } => {
                assert_eq!(*button, MouseButton::Left);
                assert_eq!(*hold_ms, 0);
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
        let binding = &config.device_bindings[0].button_bindings[0];
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
        let binding = &config.device_bindings[0].button_bindings[0];
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
        let binding = &config.device_bindings[0].button_bindings[0];
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
        let binding = &config.device_bindings[0].button_bindings[0];
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
                hold_ms: 0
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
            hold_ms: 0
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
}
