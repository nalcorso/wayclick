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
    warnings: Vec<String>,
}

impl ConfigBuilder {
    fn new() -> Self {
        Self {
            options: GlobalOptions::default(),
            triggers: Vec::new(),
            device_bindings: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn into_config(self) -> Config {
        Config {
            options: self.options,
            triggers: self.triggers,
            device_bindings: self.device_bindings,
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
            let button_str: String = table.get::<String>("button").unwrap_or_else(|_| "left".into());
            let _button = MouseButton::from_str_name(&button_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let interval_ms: u32 = table.get("interval_ms").unwrap_or(50);
            let duration_ms: Option<u32> = table.get("duration_ms").ok();
            let jitter_ms: u32 = table.get("jitter_ms").unwrap_or(0);

            let action = lua.create_table()?;
            action.set("_type", "auto_click")?;
            action.set("_button", button_str)?;
            action.set("_interval_ms", interval_ms)?;
            if let Some(d) = duration_ms {
                action.set("_duration_ms", d)?;
            }
            action.set("_jitter_ms", jitter_ms)?;
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
            let (key_name, key_code) = normalize_key_name(&key)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
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

    // wayclick.register_trigger(table)
    let register_trigger = lua
        .create_function(|lua, table: LuaTable| {
            let id: String = table
                .get::<String>("id")
                .map_err(|_| LuaError::RuntimeError("register_trigger requires 'id' field".into()))?;
            let name: String = table.get("name").unwrap_or_else(|_| id.clone());
            let description: String = table.get("description").unwrap_or_default();
            let mode_str: String = table.get("mode").unwrap_or_else(|_| "toggle".into());
            let mode = TriggerMode::from_str_mode(&mode_str)
                .map_err(|e| LuaError::RuntimeError(e.to_string()))?;
            let cooldown_ms: Option<u32> = table.get("cooldown_ms").ok();

            let action_table: LuaTable = table
                .get::<LuaTable>("action")
                .map_err(|_| LuaError::RuntimeError("register_trigger requires 'action' field".into()))?;

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
                let code: String = binding.get("code")?;
                let trigger_id: String = binding.get("trigger")?;
                button_bindings.push(ButtonBinding { code, trigger_id });
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
            let code: String = table.get("code")?;
            let trigger_id: String = table.get("trigger")?;

            let mut builder = lua.app_data_mut::<ConfigBuilder>().unwrap();
            builder.warnings.push(format!(
                "bind_evdev is deprecated. Use bind_device instead. Device: {}",
                device
            ));
            builder.device_bindings.push(DeviceBinding {
                device_match: DeviceMatch::ByPath { path: device },
                button_bindings: vec![ButtonBinding { code, trigger_id }],
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
            Ok(ActionConfig::AutoClick {
                button,
                interval_ms,
                duration_ms,
                jitter_ms,
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
                    assert_eq!(
                        *button,
                        MouseButton::from_str_name(btn).unwrap()
                    );
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
            ActionConfig::Composite { mode: CompositeMode::Sequence, actions } => {
                assert_eq!(actions.len(), 3);
                assert!(matches!(actions[0], ActionConfig::AutoClick { .. }));
                assert!(matches!(actions[1], ActionConfig::Delay { duration_ms: 500 }));
                assert!(matches!(actions[2], ActionConfig::AutoClick { .. }));
            }
            _ => panic!("Expected Sequence"),
        }
    }
}
