// SPDX-License-Identifier: MIT
//! `wayclick-recorder` — record macros against a running wayclickd.

use clap::Parser;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use wayclick_recorder::{recorder, Cli};

/// Cancel flag flipped by SIGINT/SIGTERM. A plain `static` (no `OnceLock`,
/// no allocation) is the only construct fully guaranteed async-signal-safe
/// inside `extern "C"` handlers.
static CANCEL: AtomicBool = AtomicBool::new(false);

fn main() -> ExitCode {
    let cli = Cli::parse();

    let stop_key = match cli.validate() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wayclick-recorder: {}", e);
            return ExitCode::from(2);
        }
    };

    let socket_path = wayclick_ipc_client::socket::default_socket_path();
    if !socket_path.exists() {
        eprintln!(
            "wayclick-recorder: daemon socket not found at {} — is wayclickd running?",
            socket_path.display()
        );
        return ExitCode::from(3);
    }

    install_sigint_handler();

    let result = with_output(cli.output.as_deref(), |w| {
        recorder::run(&cli, stop_key, socket_path, w, &CANCEL)
    });

    match result {
        Ok(summary) => {
            if !cli.quiet {
                eprintln!(
                    "wayclick-recorder: done ({} Lua statement{} emitted{})",
                    summary.statements_emitted,
                    if summary.statements_emitted == 1 {
                        ""
                    } else {
                        "s"
                    },
                    if summary.stopped_by_signal {
                        ", interrupted"
                    } else {
                        ""
                    }
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("wayclick-recorder: {}", e);
            ExitCode::from(1)
        }
    }
}

/// Runs `f` with a writer pointing at stdout or the requested output file.
fn with_output<F, T>(path: Option<&std::path::Path>, f: F) -> T
where
    F: FnOnce(Box<dyn Write>) -> T,
{
    match path {
        Some(p) if p.as_os_str() != "-" => match File::create(p) {
            Ok(file) => f(Box::new(BufWriter::new(file))),
            Err(e) => {
                eprintln!("wayclick-recorder: cannot open {}: {}", p.display(), e);
                std::process::exit(4);
            }
        },
        _ => f(Box::new(BufWriter::new(io::stdout().lock()))),
    }
}

/// Best-effort SIGINT/SIGTERM handler that flips `CANCEL`. The recorder
/// polls this flag inside its event loop. The handler does nothing but
/// a single relaxed atomic store, which is guaranteed async-signal-safe.
fn install_sigint_handler() {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

    extern "C" fn handler(_: nix::libc::c_int) {
        CANCEL.store(true, Ordering::Relaxed);
    }

    let action = SigAction::new(
        SigHandler::Handler(handler),
        SaFlags::empty(),
        SigSet::empty(),
    );
    // SAFETY: installing a global signal handler is process-wide and
    // unavoidable. The handler body only performs an atomic store on a
    // 'static AtomicBool, which is async-signal-safe.
    unsafe {
        let _ = sigaction(Signal::SIGINT, &action);
        let _ = sigaction(Signal::SIGTERM, &action);
    }
}
