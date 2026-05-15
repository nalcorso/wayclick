// SPDX-License-Identifier: MIT
//! Window-match state machine + IPC event loop.
//!
//! Connects to `wayclickd` via [`AsyncClient`], filters input events down
//! to the period during which a target window is focused, and feeds them
//! into the [`Emitter`].

use crate::cli::Cli;
use crate::emitter::{CapturedEvent, Emitter};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wayclick_ipc_client::{AsyncClient, FocusedWindow, IpcMessage, MonitorInfo, SyncClient};

/// Errors surfaced by the event loop. Distinct variants help main() print
/// actionable messages.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("IPC error: {0}")]
    Ipc(#[from] wayclick_ipc_client::IpcError),
    #[error("daemon socket not found: {0}")]
    SocketResolve(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("recorder timed out waiting for daemon handshake")]
    HandshakeTimeout,
}

/// Targeting rules derived from the CLI.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub any_window: bool,
    pub window: Vec<String>,
    pub app_id: Vec<String>,
    pub title: Vec<String>,
}

impl TargetSpec {
    /// Returns `true` if the given (possibly absent) focused window matches
    /// the target spec.
    ///
    /// Matching is **case-insensitive** substring matching against
    /// `app_id` and/or `title` depending on which flag the user supplied.
    /// Case-insensitivity matches what users typically expect from a
    /// "what is this window?" check and avoids surprises with
    /// CapitalisedTitles.
    pub fn matches(&self, win: Option<&FocusedWindow>) -> bool {
        if self.any_window {
            return true;
        }
        let win = match win {
            Some(w) => w,
            None => return false,
        };
        let app_id = win.app_id.to_lowercase();
        let title = win.title.to_lowercase();

        let needle = |s: &String| s.to_lowercase();

        let any_window_match = self
            .window
            .iter()
            .any(|p| app_id.contains(&needle(p)) || title.contains(&needle(p)));
        let any_app_match = self.app_id.iter().any(|p| app_id.contains(&needle(p)));
        let any_title_match = self.title.iter().any(|p| title.contains(&needle(p)));

        any_window_match || any_app_match || any_title_match
    }
}

/// Result of one recording session, returned to the caller for reporting.
pub struct RecordingSummary {
    pub statements_emitted: u32,
    pub stopped_by_signal: bool,
}

/// Runs one recording session start-to-finish. Blocks until the stop key is
/// observed, the channel is closed, or `cancel` is set.
pub fn run<W: Write>(
    cli: &Cli,
    stop_key_code: u16,
    socket_path: PathBuf,
    out: W,
    cancel: &'static AtomicBool,
) -> Result<RecordingSummary, RecorderError> {
    let target = TargetSpec {
        any_window: cli.any_window,
        window: cli.window.clone(),
        app_id: cli.app_id.clone(),
        title: cli.title.clone(),
    };
    let filter = cli.filter();
    let format = cli.output_format();
    let coord_space = cli.coord_space_mode();
    let quiet = cli.quiet;

    if cli.verbose && !quiet {
        eprintln!("wayclick-recorder: connecting to {}", socket_path.display());
    }

    // Probe cursor-position support once up-front so we can warn the user
    // (a single check; per-event queries are still attempted at press time
    // because backends are dynamic).
    let cursor_supported = matches!(SyncClient::get_cursor_position(&socket_path), Ok(Some(_)));
    if !cursor_supported && !quiet {
        eprintln!(
            "wayclick-recorder: note — daemon does not report cursor position; \
             mouse clicks will be emitted as keystrokes rather than click_at()"
        );
    }

    // Query monitor layout up-front. Used by the emitter to classify each
    // click into a monitor and emit monitor-local coordinates. Falls back
    // to global coords when the daemon doesn't support `get_monitors`.
    let monitors: Vec<MonitorInfo> = match coord_space {
        crate::emitter::CoordSpace::Monitor => match SyncClient::get_monitors(&socket_path) {
            Ok(Some(m)) if !m.is_empty() => m,
            Ok(_) => {
                if !quiet {
                    eprintln!(
                        "wayclick-recorder: note — daemon does not report monitor layout; \
                         click_at output will use global coordinates"
                    );
                }
                Vec::new()
            }
            Err(e) => {
                if !quiet {
                    eprintln!(
                        "wayclick-recorder: warning — monitor query failed ({}); \
                         falling back to global coordinates",
                        e
                    );
                }
                Vec::new()
            }
        },
        crate::emitter::CoordSpace::Global => Vec::new(),
    };

    let client = AsyncClient::connect(socket_path.clone())?;
    let mut emitter = Emitter::with_coords(out, filter, format, coord_space, monitors);
    emitter.begin()?;

    // Wait for initial Connected event with a short timeout.
    let connected_deadline = std::time::Instant::now() + Duration::from_secs(5);
    let focused: Option<FocusedWindow>;
    'connect: loop {
        if std::time::Instant::now() >= connected_deadline {
            return Err(RecorderError::HandshakeTimeout);
        }
        if cancel.load(Ordering::Relaxed) {
            emitter.end()?;
            return Ok(RecordingSummary {
                statements_emitted: emitter.statement_count(),
                stopped_by_signal: true,
            });
        }
        match client.recv_timeout(Duration::from_millis(100))? {
            Some(IpcMessage::Connected { initial_focus, .. }) => {
                focused = initial_focus;
                break 'connect;
            }
            Some(_) | None => continue,
        }
    }

    if cli.verbose && !quiet {
        match &focused {
            Some(w) => eprintln!(
                "wayclick-recorder: focused window: app_id={:?} title={:?}",
                w.app_id, w.title
            ),
            None => eprintln!("wayclick-recorder: no focused window"),
        }
    }

    let mut recording_active = target.matches(focused.as_ref());
    if !quiet {
        if recording_active {
            eprintln!("wayclick-recorder: RECORDING (press the stop key to finish)");
        } else {
            eprintln!(
                "wayclick-recorder: idle — waiting for target window to gain focus \
                 (press stop key to abort)"
            );
        }
    }

    let mut stopped_by_signal = false;

    loop {
        if cancel.load(Ordering::Relaxed) {
            stopped_by_signal = true;
            break;
        }
        // Block up to 100ms waiting for the next event. This still lets us
        // poll the cancel flag promptly but avoids the busy-spin that
        // `try_recv + sleep` produces (which wakes every 20ms even when
        // idle).
        let msg = match client.recv_timeout(Duration::from_millis(100))? {
            Some(m) => m,
            None => continue,
        };

        match msg {
            IpcMessage::FocusChanged(win) => {
                let now = target.matches(win.as_ref());
                if now != recording_active {
                    recording_active = now;
                    if !quiet {
                        if now {
                            eprintln!("wayclick-recorder: target focused — RECORDING");
                        } else {
                            eprintln!("wayclick-recorder: target unfocused — IDLE");
                        }
                    }
                }
            }
            IpcMessage::RawInput { code, value, .. } => {
                // Watch for stop key regardless of recording state — fires on press only.
                if code == stop_key_code && value == 1 {
                    if !quiet {
                        eprintln!("wayclick-recorder: stop key received");
                    }
                    break;
                }
                if !recording_active {
                    continue;
                }
                let ts = now_ms();
                // For mouse button presses, query the daemon for current cursor.
                let cursor = if value == 1 && crate::keymap::click_at_button(code).is_some() {
                    SyncClient::get_cursor_position(&socket_path).unwrap_or_default()
                } else {
                    None
                };
                emitter.push(
                    &CapturedEvent::Input {
                        code,
                        value,
                        timestamp_ms: ts,
                    },
                    cursor,
                )?;
            }
            IpcMessage::ScrollReceived { delta_x, delta_y } => {
                if !recording_active {
                    continue;
                }
                emitter.push(
                    &CapturedEvent::Scroll {
                        delta_x,
                        delta_y,
                        timestamp_ms: now_ms(),
                    },
                    None,
                )?;
            }
            IpcMessage::Disconnected => {
                if !quiet {
                    eprintln!(
                        "wayclick-recorder: lost connection to daemon — finalising transcript"
                    );
                }
                break;
            }
            _ => {}
        }
    }

    emitter.end()?;

    Ok(RecordingSummary {
        statements_emitted: emitter.statement_count(),
        stopped_by_signal,
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wayclick_ipc_client::FocusedWindow;

    fn win(app_id: &str, title: &str) -> FocusedWindow {
        FocusedWindow {
            app_id: app_id.to_string(),
            title: title.to_string(),
            process_name: None,
            backend: "test".to_string(),
            xwayland: false,
        }
    }

    #[test]
    fn any_window_matches_anything() {
        let t = TargetSpec {
            any_window: true,
            window: vec![],
            app_id: vec![],
            title: vec![],
        };
        assert!(t.matches(Some(&win("foo", "bar"))));
        assert!(t.matches(None));
    }

    #[test]
    fn window_matches_either_field() {
        let t = TargetSpec {
            any_window: false,
            window: vec!["rev".to_string()],
            app_id: vec![],
            title: vec![],
        };
        assert!(t.matches(Some(&win("revolution-idle", "Game"))));
        assert!(t.matches(Some(&win("foo", "Revelations"))));
        assert!(!t.matches(Some(&win("foo", "bar"))));
        assert!(!t.matches(None));
    }

    #[test]
    fn app_id_matches_only_app_id() {
        let t = TargetSpec {
            any_window: false,
            window: vec![],
            app_id: vec!["firefox".to_string()],
            title: vec![],
        };
        assert!(t.matches(Some(&win("firefox", "Anywhere"))));
        assert!(!t.matches(Some(&win("kitty", "firefox is in title"))));
    }

    #[test]
    fn title_matches_only_title() {
        let t = TargetSpec {
            any_window: false,
            window: vec![],
            app_id: vec![],
            title: vec!["Inbox".to_string()],
        };
        assert!(t.matches(Some(&win("firefox", "Inbox · Gmail"))));
        assert!(!t.matches(Some(&win("Inbox", "Other"))));
    }
}
