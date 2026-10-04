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

const USAGE: &str = "usage: atlas-installer-helper [--list | --plan <disk> erase|free-space [locale] [keymap] [wifi-uuid] [none|tpm|tpm-pin|password]]";

fn fail(e: impl std::fmt::Display) -> ExitCode {
    eprintln!("atlas-installer-helper: {e}");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    // Few threads: with mlockall each thread stack is locked in RAM, and the
    // live ISO's overlay is RAM too.
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(16)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(format!("cannot start the async runtime: {e}")),
    };
    rt.block_on(run())
}

async fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let env = Env::system();
    match args.as_slice() {
        [] => {
            // The live system runs from its USB stick or disc, and people
            // take it out before they press Restart. Pages not yet read in
            // can't be read then, so read in all of the helper now and keep
            // it: restarting (install::restart) then needs nothing from it.
            // Then MCL_FUTURE | MCL_ONFAULT, with no MCL_CURRENT: what is
            // mapped now stays populated and locked, later mappings (thread
            // stacks, heap) lock only as they are touched, so the secrets it
            // handles later still never reach swap without locking every
            // untouched stack page.
            // No core dump or ptrace by other processes: the helper holds
            // disk passwords. A failure is logged; the helper carries on.
            // SAFETY: PR_SET_DUMPABLE takes an integer flag
            if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
                eprintln!(
                    "atlas-installer-helper: WARNING: can't clear the dumpable flag, a crash may leave a core dump: {}",
                    std::io::Error::last_os_error()
                );
            }
            // SAFETY: mlockall takes only flags
            if unsafe { libc::mlockall(libc::MCL_CURRENT) } != 0 {
                eprintln!(
                    "atlas-installer-helper: can't lock itself in memory: {}",
                    std::io::Error::last_os_error()
                );
            }
            // SAFETY: as above
            if unsafe { libc::mlockall(libc::MCL_FUTURE | libc::MCL_ONFAULT) } != 0 {
                eprintln!(
                    "atlas-installer-helper: can't lock later memory: {}",
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
        ["--plan", disk, mode, rest @ ..] if rest.len() <= 4 => {
            let get = |i: usize, default| rest.get(i).copied().unwrap_or(default);
            let req = match Request::new(
                disk,
                "",
                mode,
                get(0, "en_US.UTF-8"),
                get(1, "us"),
                get(2, ""),
                get(3, "none"),
                // only the plan is shown: a stand-in for the password or PIN
                if matches!(get(3, "none"), "password" | "tpm-pin") {
                    "plan-only secret"
                } else {
                    ""
                },
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
