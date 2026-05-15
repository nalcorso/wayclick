// SPDX-License-Identifier: MIT
//! Command-line interface for `wayclick-recorder`.

use clap::Parser;
use std::path::PathBuf;

/// Macro recorder for wayclick.
///
/// Connects to a running wayclickd, listens for input events while a
/// target window is focused, and emits a stream of Lua snippets that
/// replay the captured sequence. Stop the recording by pressing the
/// configured stop key (default: Pause).
///
/// The output is line-oriented Lua suitable for pasting into existing
/// wayclick scripts. It is intentionally NOT a complete self-contained
/// program by default — use `--format script` to wrap in a stub trigger.
#[derive(Debug, Parser)]
#[command(
    name = "wayclick-recorder",
    version,
    about = "Record input events scoped to a focused window and emit replay-able Lua snippets.",
    long_about = None
)]
pub struct Cli {
    // ---- Targeting --------------------------------------------------------
    /// Substring match against the focused window's `app_id` OR `title`.
    /// May be repeated; any match counts.
    #[arg(long = "window", value_name = "PATTERN")]
    pub window: Vec<String>,

    /// Substring match against the focused window's `app_id` only.
    #[arg(long = "app-id", value_name = "PATTERN")]
    pub app_id: Vec<String>,

    /// Substring match against the focused window's `title` only.
    #[arg(long = "title", value_name = "PATTERN")]
    pub title: Vec<String>,

    /// Display/output filtering. Accepted for forward compatibility but
    /// rejected at runtime: the daemon does not yet track per-display focus.
    #[arg(long = "display", value_name = "NAME")]
    pub display: Option<String>,

    /// Record without window filtering — every input event reaches the
    /// emitter regardless of focus. Cannot be combined with `--window`,
    /// `--app-id`, or `--title`.
    #[arg(long = "any-window")]
    pub any_window: bool,

    // ---- Filtering --------------------------------------------------------
    /// Drop key press/release events (KEY_* codes).
    #[arg(long = "no-keys")]
    pub no_keys: bool,

    /// Drop mouse button events (BTN_* codes).
    #[arg(long = "no-buttons")]
    pub no_buttons: bool,

    /// Emit mouse buttons as keystrokes (e.g. `keystroke({ key = "BTN_LEFT" })`)
    /// instead of `click_at({ x, y, ... })`. Useful for binds that should
    /// fire regardless of cursor location.
    #[arg(long = "no-clicks")]
    pub no_clicks: bool,

    /// Drop wheel events.
    #[arg(long = "no-scroll")]
    pub no_scroll: bool,

    /// Drop inter-event `wayclick.delay({...})` lines.
    #[arg(long = "no-delays")]
    pub no_delays: bool,

    /// Coalesce inter-event delays shorter than this many milliseconds into 0
    /// (i.e. don't emit them). Default: 0 (keep every delay).
    #[arg(long = "min-delay-ms", value_name = "N", default_value_t = 0)]
    pub min_delay_ms: u32,

    // ---- Stop key ---------------------------------------------------------
    /// Key that ends the recording. Examples: `pause`, `scroll_lock`, `f10`.
    /// Modifier combos are not yet supported in sentinel mode.
    #[arg(long = "stop-key", value_name = "KEY", default_value = "pause")]
    pub stop_key: String,

    /// How the stop key is detected. `sentinel` (default) subscribes to the
    /// event stream and watches for the key — note that the keypress will
    /// also reach the foreground app. `exclusive` mode (planned) would have
    /// the daemon grab the key so it's swallowed; not yet implemented and
    /// will error if selected.
    #[arg(
        long = "stop-mode",
        value_name = "MODE",
        default_value = "sentinel",
        value_parser = ["sentinel", "exclusive"]
    )]
    pub stop_mode: String,

    // ---- Output -----------------------------------------------------------
    /// Output format. `raw` (default) writes bare Lua statements; `script`
    /// wraps the block in a `wayclick.register_trigger` skeleton with TODO
    /// placeholders.
    #[arg(
        long = "format",
        value_name = "FMT",
        default_value = "raw",
        value_parser = ["raw", "script"]
    )]
    pub format: String,

    /// Coordinate space for `click_at` output.
    ///
    /// `monitor` (default): emit `click_at({ x = local_x, y = local_y,
    /// monitor = "DP-2" })`, where `(local_x, local_y)` are the cursor's
    /// position **relative to its containing monitor's top-left**.
    ///
    /// `global`: emit `click_at({ x = global_x, y = global_y })` in the
    /// compositor's global pixel layout. Required for scripts that need
    /// to span multiple monitors.
    ///
    /// On compositors without monitor-introspection support (anything
    /// other than Hyprland today), monitor-mode falls back to `global`
    /// with a one-time warning.
    #[arg(
        long = "coord-space",
        value_name = "SPACE",
        default_value = "monitor",
        value_parser = ["monitor", "global"]
    )]
    pub coord_space: String,

    /// Output file (`-` or omitted = stdout).
    #[arg(long = "output", short = 'o', value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Print status messages to stderr.
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Suppress non-fatal stderr output.
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,
}

/// Errors surfaced by argument validation. Distinct from runtime errors so
/// the binary can exit with a clean message before doing any IPC.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("--display is not yet supported: wayclickd does not track per-display focus. Tracking issue: https://github.com/nalcorso/wayclick/issues")]
    DisplayUnsupported,

    #[error("at least one of --window, --app-id, --title, or --any-window is required")]
    NoTargetSpecified,

    #[error("--any-window cannot be combined with --window/--app-id/--title")]
    ConflictingTargets,

    #[error("--stop-key '{0}' is not a recognised key name")]
    InvalidStopKey(String),

    #[error(
        "--stop-mode exclusive is not yet implemented in this build; use --stop-mode sentinel"
    )]
    ExclusiveStopNotImplemented,
}

impl Cli {
    /// Validates argument combinations and returns either the resolved
    /// stop-key evdev code or a [`CliError`]. Centralised here so `main`
    /// stays trivial.
    pub fn validate(&self) -> Result<u16, CliError> {
        if self.display.is_some() {
            return Err(CliError::DisplayUnsupported);
        }
        let has_target =
            !self.window.is_empty() || !self.app_id.is_empty() || !self.title.is_empty();
        if !has_target && !self.any_window {
            return Err(CliError::NoTargetSpecified);
        }
        if has_target && self.any_window {
            return Err(CliError::ConflictingTargets);
        }
        if self.stop_mode == "exclusive" {
            return Err(CliError::ExclusiveStopNotImplemented);
        }
        let code = crate::keymap::parse_stop_key(&self.stop_key)
            .ok_or_else(|| CliError::InvalidStopKey(self.stop_key.clone()))?;
        Ok(code)
    }

    /// Returns the configured filter set derived from CLI flags.
    pub fn filter(&self) -> crate::filter::FilterSet {
        crate::filter::FilterSet {
            no_keys: self.no_keys,
            no_buttons: self.no_buttons,
            no_clicks: self.no_clicks,
            no_scroll: self.no_scroll,
            no_delays: self.no_delays,
            min_delay_ms: self.min_delay_ms,
        }
    }

    /// Resolved output format.
    pub fn output_format(&self) -> crate::emitter::OutputFormat {
        match self.format.as_str() {
            "script" => crate::emitter::OutputFormat::Script,
            _ => crate::emitter::OutputFormat::Raw,
        }
    }

    /// Resolved coordinate-space mode.
    pub fn coord_space_mode(&self) -> crate::emitter::CoordSpace {
        match self.coord_space.as_str() {
            "global" => crate::emitter::CoordSpace::Global,
            _ => crate::emitter::CoordSpace::Monitor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["wayclick-recorder"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).expect("parse")
    }

    #[test]
    fn display_is_rejected() {
        let c = parse(&["--display", "DP-2", "--any-window"]);
        assert!(matches!(c.validate(), Err(CliError::DisplayUnsupported)));
    }

    #[test]
    fn no_target_is_rejected() {
        let c = parse(&[]);
        assert!(matches!(c.validate(), Err(CliError::NoTargetSpecified)));
    }

    #[test]
    fn any_window_and_target_conflict() {
        let c = parse(&["--any-window", "--window", "foo"]);
        assert!(matches!(c.validate(), Err(CliError::ConflictingTargets)));
    }

    #[test]
    fn invalid_stop_key_rejected() {
        let c = parse(&["--any-window", "--stop-key", "nonsense"]);
        assert!(matches!(c.validate(), Err(CliError::InvalidStopKey(_))));
    }

    #[test]
    fn default_stop_key_is_pause() {
        let c = parse(&["--any-window"]);
        assert_eq!(c.validate().unwrap(), 119); // KEY_PAUSE
    }

    #[test]
    fn exclusive_mode_not_yet_implemented() {
        let c = parse(&["--any-window", "--stop-mode", "exclusive"]);
        assert!(matches!(
            c.validate(),
            Err(CliError::ExclusiveStopNotImplemented)
        ));
    }
}
