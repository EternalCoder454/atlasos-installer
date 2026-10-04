//! `atlas-installer-helper`: the root half of Atlas Installer. Started by
//! D-Bus on the system bus (`net.eterneon.atlas.InstallerHelper`); polkit
//! decides who may list disks, install and restart.
//!
//! For testing in the live session, as root:
//!
//!     atlas-installer-helper --list
//!     atlas-installer-helper --plan <disk> erase|free-space [locale] [keymap] [wifi-uuid]
//!
//! `--list` prints what ListDisks returns; `--plan` prints what Install
//! would do. Neither writes to any disk (EFI partitions are mounted
//! read-only to look at them).

mod install;
mod run;
mod service;

use std::process::ExitCode;
use std::sync::Arc;

use install::{Env, Request};
use run::SystemRunner;
use service::{IDLE_TIMEOUT, Service, serve};

const USAGE: &str = "usage: atlas-installer-helper [--list | --plan <disk> erase|free-space [locale] [keymap] [wifi-uuid]]";

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("atlas-installer-helper: {e}");
    ExitCode::FAILURE
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let env = Env::system();
    match args.as_slice() {
        [] => {
            // The live system runs from its USB stick or disc, and people
            // take it out before they press Restart. Pages not yet read in
            // can't be read then, so read in all of the helper now and keep
            // it: restarting (install::restart) then needs nothing from it.
            // SAFETY: mlockall takes only flags
            if unsafe { libc::mlockall(libc::MCL_CURRENT) } != 0 {
                eprintln!(
                    "atlas-installer-helper: can't lock itself in memory: {}",
                    std::io::Error::last_os_error()
                );
            }
            let builder = match zbus::connection::Builder::system() {
                Ok(b) => b,
                Err(e) => return fail(format!("cannot reach the system bus: {e}")),
            };
            match serve(
                builder,
                Service::new(Arc::new(SystemRunner), env),
                IDLE_TIMEOUT,
            )
            .await
            {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => fail(e),
            }
        }
        ["--list" | "--plan", ..] if !rustix::process::geteuid().is_root() => {
            // as a user, mounts fail quietly and the answer looks right but isn't
            fail("run this as root")
        }
        ["--list"] => match install::list_disks(&SystemRunner, &env) {
            Ok(l) => {
                println!("{}", serde_json::to_string_pretty(&l).unwrap_or_default());
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        ["--plan", disk, mode, rest @ ..] if rest.len() <= 3 => {
            let get = |i: usize, default| rest.get(i).copied().unwrap_or(default);
            let req = match Request::new(
                disk,
                "",
                mode,
                get(0, "en_US.UTF-8"),
                get(1, "us"),
                get(2, ""),
            ) {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            match install::prepare(&SystemRunner, &env, &req) {
                Ok(p) => {
                    print!("{}", install::describe(&p, &req, &env));
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
