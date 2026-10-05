//! Running commands: absolute paths, a clean environment, a timeout, capped
//! output, and each output line handed to the caller as it arrives.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

/// Every program the helper runs, by absolute path (Fedora 42+ merged
/// /usr/sbin into /usr/bin).
pub mod bin {
    pub const BOOTC: &str = "/usr/bin/bootc";
    pub const CRYPTENROLL: &str = "/usr/bin/systemd-cryptenroll";
    pub const CRYPTSETUP: &str = "/usr/bin/cryptsetup";
    pub const EFIBOOTMGR: &str = "/usr/bin/efibootmgr";
    pub const GRUB2_MKPASSWD: &str = "/usr/bin/grub2-mkpasswd-pbkdf2";
    pub const FINDMNT: &str = "/usr/bin/findmnt";
    pub const LSBLK: &str = "/usr/bin/lsblk";
    pub const MKFS_BTRFS: &str = "/usr/bin/mkfs.btrfs";
    pub const MKFS_EXT4: &str = "/usr/bin/mkfs.ext4";
    pub const MKFS_VFAT: &str = "/usr/bin/mkfs.vfat";
    pub const MOKUTIL: &str = "/usr/bin/mokutil";
    pub const MOUNT: &str = "/usr/bin/mount";
    pub const PODMAN: &str = "/usr/bin/podman";
    pub const SETFILES: &str = "/usr/bin/setfiles";
    pub const SFDISK: &str = "/usr/bin/sfdisk";
    pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
    pub const UDEVADM: &str = "/usr/bin/udevadm";
    pub const UMOUNT: &str = "/usr/bin/umount";
    pub const WIPEFS: &str = "/usr/bin/wipefs";
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// Most output kept from one command (stdout and stderr each); past it the
/// oldest is dropped, as errors come last.
const OUTPUT_CAP: usize = 4 * 1024 * 1024;
/// Longest line kept.
const LINE_CAP: usize = 64 * 1024;
/// How long to wait for a killed command to go away. A process stuck in
/// the kernel (a failing disk) can't be killed; the helper then gives up on
/// it rather than hang with the install marked as running.
const KILL_WAIT: Duration = Duration::from_secs(10);
/// How much of stderr goes into an error message.
const ERROR_TAIL: usize = 2048;

#[derive(Clone)]
pub struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
    pub stdin: Option<String>,
    /// Environment variables that hold a secret (the runner's environment
    /// is otherwise empty). Never logged or shown by `Debug`.
    pub secret_env: Vec<(&'static str, String)>,
    pub timeout: Duration,
    /// The arguments or stdin hold a secret: the arguments are not logged
    /// (stdin never is).
    pub secret: bool,
    /// Cleanup (unmounting) that still runs while the helper shuts down.
    pub cleanup: bool,
}

impl Cmd {
    pub fn new<I, S>(program: &'static str, args: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Cmd {
            program,
            args: args.into_iter().map(Into::into).collect(),
            stdin: None,
            secret_env: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            secret: false,
            cleanup: false,
        }
    }

    pub fn stdin(mut self, s: impl Into<String>) -> Cmd {
        self.stdin = Some(s.into());
        self
    }

    /// Set a secret environment variable for this command.
    pub fn secret_env(mut self, name: &'static str, value: impl Into<String>) -> Cmd {
        self.secret_env.push((name, value.into()));
        self
    }

    pub fn timeout(mut self, t: Duration) -> Cmd {
        self.timeout = t;
        self
    }

    pub fn secret(mut self) -> Cmd {
        self.secret = true;
        self
    }

    pub fn cleanup(mut self) -> Cmd {
        self.cleanup = true;
        self
    }

    /// The program's file name.
    pub fn name(&self) -> &'static str {
        self.program.rsplit('/').next().unwrap_or(self.program)
    }

    /// For logs: the command line, or only the program if it holds a secret.
    pub fn display(&self) -> String {
        if self.secret {
            format!("{} (arguments hidden)", self.program)
        } else {
            std::iter::once(self.program.to_string())
                .chain(self.args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// Overwrite a string's bytes before it is freed. Volatile writes and a fence
/// keep the compiler from dropping them as dead stores.
pub fn wipe(s: String) {
    let mut bytes = s.into_bytes();
    for b in bytes.iter_mut() {
        // SAFETY: `b` is a valid, aligned, exclusive reference
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
    drop(bytes);
}

/// The secrets in stdin and the environment are wiped when the command goes.
impl Drop for Cmd {
    fn drop(&mut self) {
        if let Some(s) = self.stdin.take() {
            wipe(s);
        }
        for (_, v) in std::mem::take(&mut self.secret_env) {
            wipe(v);
        }
    }
}

/// Stdin and secret variables are never shown, whatever they hold.
impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = |present: bool| if present { "<hidden>" } else { "<none>" };
        f.debug_struct("Cmd")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("stdin", &hidden(self.stdin.is_some()))
            .field(
                "secret_env",
                &self.secret_env.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            )
            .field("timeout", &self.timeout)
            .field("secret", &self.secret)
            .field("cleanup", &self.cleanup)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// Exit code; `None` if a signal ended it.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// The output if it exited 0, else an error with its stderr tail.
    pub fn check(self, cmd: &Cmd) -> Result<Output, String> {
        if self.ok() {
            return Ok(self);
        }
        let text = tail(
            if self.stderr.trim().is_empty() {
                &self.stdout
            } else {
                &self.stderr
            },
            ERROR_TAIL,
        );
        let status = match self.code {
            Some(c) => format!("exit status {c}"),
            None => "stopped by a signal".into(),
        };
        Err(if text.is_empty() {
            format!("{} failed ({status})", cmd.name())
        } else {
            format!("{} failed ({status}): {text}", cmd.name())
        })
    }
}

/// The last `max` bytes of `s`, cut on a char boundary.
pub fn tail(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

/// Runs a command. `on_line` gets each line of output (stdout and stderr)
/// as `Some`, and `None` about twice a second while the command runs.
/// `Err` only if it could not be run or ran out of time.
pub trait Runner: Send + Sync {
    fn run(&self, cmd: &Cmd, on_line: &mut dyn FnMut(Option<&str>)) -> Result<Output, String>;

    /// Unmount `unmount` (deepest first), write out the disks and restart
    /// at once, through the kernel, without stopping anything else first.
    /// Returns only on failure. For when the live
    /// medium is gone: `systemctl reboot` and the clean shutdown after it
    /// need programs from the medium (see `install::restart`).
    fn restart_now(&self, unmount: &[String]) -> Result<(), String> {
        let _ = unmount;
        Err("this runner can't restart the computer".into())
    }
}

/// Run `cmd` and require exit status 0.
pub fn run(r: &dyn Runner, cmd: Cmd) -> Result<Output, String> {
    r.run(&cmd, &mut |_| {})?.check(&cmd)
}

/// Lock, ignoring poisoning (a panicking thread must not wedge the helper).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Process groups of the commands running now, and whether each is cleanup.
static RUNNING: Mutex<Option<HashMap<u32, bool>>> = Mutex::new(None);
/// Set at shutdown: commands started from now on are stopped at once,
/// except cleanup.
static CLOSING: AtomicBool = AtomicBool::new(false);

fn kill_group(pgid: u32, signal: rustix::process::Signal) {
    if let Some(pid) = i32::try_from(pgid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, signal);
    }
}

/// The process groups `terminate_running` signals: everything but cleanup.
fn to_terminate(running: &HashMap<u32, bool>) -> Vec<u32> {
    let mut v: Vec<u32> = running
        .iter()
        .filter(|(_, cleanup)| !**cleanup)
        .map(|(p, _)| *p)
        .collect();
    v.sort_unstable();
    v
}

/// Ask every running command to stop (shutdown), except cleanup: an
/// unmount that is already running is left to finish.
pub fn terminate_running() {
    CLOSING.store(true, Ordering::Release);
    let pgids = to_terminate(lock(&RUNNING).as_ref().unwrap_or(&HashMap::new()));
    for pgid in pgids {
        kill_group(pgid, rustix::process::Signal::TERM);
    }
}

/// The real runner.
pub struct SystemRunner;

enum Event {
    Line(String),
    Closed,
}

fn read_lines(r: impl Read, tx: mpsc::Sender<(bool, Event)>, is_err: bool) {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // At most LINE_CAP + 1 bytes are held per line; the rest of a longer
        // line is read and dropped up to its newline.
        match (&mut r)
            .take(LINE_CAP as u64 + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if buf.len() > LINE_CAP && buf.last() != Some(&b'\n') {
                    // over the cap with no newline yet: skip to the next one
                    let mut skip = Vec::new();
                    loop {
                        skip.clear();
                        match (&mut r).take(LINE_CAP as u64).read_until(b'\n', &mut skip) {
                            Ok(0) | Err(_) => break,
                            Ok(_) if skip.last() == Some(&b'\n') => break,
                            Ok(_) => {}
                        }
                    }
                }
                buf.truncate(LINE_CAP);
                let line = String::from_utf8_lossy(&buf)
                    .trim_end_matches(['\n', '\r'])
                    .to_string();
                if tx.send((is_err, Event::Line(line))).is_err() {
                    break;
                }
            }
        }
    }
    let _ = tx.send((is_err, Event::Closed));
}

impl Runner for SystemRunner {
    fn restart_now(&self, unmount: &[String]) -> Result<(), String> {
        for dir in unmount {
            let Ok(c) = std::ffi::CString::new(dir.as_str()) else {
                continue;
            };
            // SAFETY: c is a valid C string for both calls. A lazy unmount
            // if busy: with nothing using it, it is written out at once.
            unsafe {
                if libc::umount2(c.as_ptr(), 0) != 0 {
                    libc::umount2(c.as_ptr(), libc::MNT_DETACH);
                }
            }
        }
        // SAFETY: sync and reboot take no pointers; reboot returns only if
        // it failed
        unsafe {
            libc::sync();
            libc::reboot(libc::RB_AUTOBOOT);
        }
        Err(format!(
            "the kernel refused to restart: {}",
            std::io::Error::last_os_error()
        ))
    }

    fn run(&self, cmd: &Cmd, on_line: &mut dyn FnMut(Option<&str>)) -> Result<Output, String> {
        let mut child = Command::new(cmd.program)
            .args(&cmd.args)
            .env_clear()
            .env("PATH", "/usr/bin")
            .env("LANG", "C.UTF-8")
            .envs(cmd.secret_env.iter().map(|(k, v)| (*k, v.as_str())))
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", cmd.program))?;
        let pgid = child.id();
        lock(&RUNNING)
            .get_or_insert_with(HashMap::new)
            .insert(pgid, cmd.cleanup);
        if CLOSING.load(Ordering::Acquire) && !cmd.cleanup {
            kill_group(pgid, rustix::process::Signal::TERM);
        }
        let result = supervise(&mut child, pgid, cmd, on_line);
        if let Some(set) = lock(&RUNNING).as_mut() {
            set.remove(&pgid);
        }
        result
    }
}

fn supervise(
    child: &mut std::process::Child,
    pgid: u32,
    cmd: &Cmd,
    on_line: &mut dyn FnMut(Option<&str>),
) -> Result<Output, String> {
    // the writer outlives this call, so it needs its own copy; it wipes it
    if let Some(mut pipe) = child.stdin.take()
        && let Some(input) = cmd.stdin.clone()
    {
        std::thread::spawn(move || {
            let _ = pipe.write_all(input.as_bytes());
            wipe(input);
        });
    }
    let (tx, rx) = mpsc::channel();
    let out = child.stdout.take().ok_or("no stdout")?;
    let err = child.stderr.take().ok_or("no stderr")?;
    let tx2 = tx.clone();
    std::thread::spawn(move || read_lines(out, tx, false));
    std::thread::spawn(move || read_lines(err, tx2, true));

    let deadline = Instant::now() + cmd.timeout;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut open = 2;
    let mut status = None;
    let mut grace_end = None;
    let keep = |is_err: bool, line: &str, stdout: &mut String, stderr: &mut String| {
        let buf = if is_err { stderr } else { stdout };
        buf.push_str(line);
        buf.push('\n');
        if buf.len() > OUTPUT_CAP {
            let mut cut = buf.len() - OUTPUT_CAP / 2;
            while !buf.is_char_boundary(cut) {
                cut += 1;
            }
            buf.drain(..cut);
        }
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok((is_err, Event::Line(line))) => {
                on_line(Some(&line));
                keep(is_err, &line, &mut stdout, &mut stderr);
            }
            Ok((_, Event::Closed)) => open -= 1,
            Err(mpsc::RecvTimeoutError::Timeout) => on_line(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => open = 0,
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(s)) => {
                    status = Some(s);
                    // a grandchild may hold the pipes open
                    grace_end = Some(Instant::now() + Duration::from_secs(2));
                }
                Ok(None) if Instant::now() >= deadline => {
                    kill_group(pgid, rustix::process::Signal::KILL);
                    let _ = child.kill();
                    let gone = Instant::now() + KILL_WAIT;
                    while !matches!(child.try_wait(), Ok(Some(_)) | Err(_)) && Instant::now() < gone
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    return Err(format!(
                        "{} did not finish within {} s and was stopped",
                        cmd.name(),
                        cmd.timeout.as_secs()
                    ));
                }
                Ok(None) => {}
                Err(e) => return Err(format!("waiting for {} failed: {e}", cmd.name())),
            }
        }
        if let Some(s) = status
            && (open == 0 || grace_end.is_some_and(|g| Instant::now() >= g))
        {
            if open > 0 {
                kill_group(pgid, rustix::process::Signal::KILL);
            }
            // lines read but not yet handled, from both streams
            while let Ok((is_err, ev)) = rx.try_recv() {
                if let Event::Line(line) = ev {
                    on_line(Some(&line));
                    keep(is_err, &line, &mut stdout, &mut stderr);
                }
            }
            return Ok(Output {
                code: s.code(),
                stdout,
                stderr,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sh;
    impl Sh {
        fn cmd(script: &str) -> Cmd {
            Cmd::new("/bin/sh", ["-c", script])
        }
    }

    #[test]
    fn a_huge_line_is_cut_without_being_held_and_the_next_line_survives() {
        let mut input = vec![b'x'; LINE_CAP * 5 + 17];
        input.extend_from_slice(b"\nshort\r\n");
        input.extend(vec![b'y'; LINE_CAP + 1]);
        input.extend_from_slice(b"\nlast");
        let (tx, rx) = mpsc::channel();
        read_lines(std::io::Cursor::new(input), tx, false);
        let lines: Vec<String> = rx
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Line(l) => Some(l),
                Event::Closed => None,
            })
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "x".repeat(LINE_CAP));
        assert_eq!(lines[1], "short");
        assert_eq!(lines[2], "y".repeat(LINE_CAP));
        assert_eq!(lines[3], "last");
    }

    #[test]
    fn a_newline_free_stream_is_one_capped_line() {
        let (tx, rx) = mpsc::channel();
        read_lines(std::io::repeat(b'z').take(LINE_CAP as u64 * 40), tx, true);
        let lines: Vec<_> = rx.iter().collect();
        assert_eq!(lines.len(), 2);
        assert!(matches!(&lines[0].1, Event::Line(l) if l.len() == LINE_CAP));
        assert!(matches!(lines[1].1, Event::Closed));
    }

    #[test]
    fn lines_arrive_as_printed_and_output_is_kept() {
        let mut lines = Vec::new();
        let out = SystemRunner
            .run(&Sh::cmd("echo one; echo two >&2; printf three"), &mut |l| {
                if let Some(l) = l {
                    lines.push(l.to_string())
                }
            })
            .unwrap();
        lines.sort();
        assert_eq!(lines, ["one", "three", "two"]);
        assert_eq!(out.stdout, "one\nthree\n");
        assert_eq!(out.stderr, "two\n");
        assert!(out.ok());
    }

    #[test]
    fn environment_is_clean_and_stdin_is_fed() {
        let out = SystemRunner
            .run(
                &Sh::cmd("echo \"$HOME|$PATH\"; cat").stdin("fed\n"),
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(out.stdout, "|/usr/bin\nfed\n");
    }

    #[test]
    fn failures_carry_stderr() {
        let c = Sh::cmd("echo boom >&2; exit 3");
        let e = run(&SystemRunner, c).unwrap_err();
        assert_eq!(e, "sh failed (exit status 3): boom");
    }

    #[test]
    fn timeout_kills_the_group() {
        let started = Instant::now();
        let c = Sh::cmd("sleep 30 & sleep 30").timeout(Duration::from_millis(300));
        let e = SystemRunner.run(&c, &mut |_| {}).unwrap_err();
        assert!(e.contains("did not finish"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_end_of_long_output_is_kept() {
        // 6 MiB of stdout, then the line that explains the failure
        let c = Sh::cmd(
            "head -c 6291456 /dev/zero | tr '\\0' 'x' | fold -w 1000; echo; echo the-real-error >&2; echo last; exit 1",
        );
        let out = SystemRunner.run(&c, &mut |_| {}).unwrap();
        assert!(out.stdout.len() <= OUTPUT_CAP);
        assert!(out.stdout.ends_with("last\n"));
        assert!(out.check(&c).unwrap_err().contains("the-real-error"));
    }

    #[test]
    fn stdin_larger_than_a_pipe_is_fed() {
        let input = "y\n".repeat(200_000);
        let out = SystemRunner.run(&Sh::cmd("wc -l"), &mut |_| {}).map(|_| ());
        assert!(out.is_ok());
        let out = SystemRunner
            .run(&Sh::cmd("wc -l").stdin(input), &mut |_| {})
            .unwrap();
        assert_eq!(out.stdout.trim(), "200000");
    }

    #[test]
    fn shutdown_leaves_a_running_cleanup_alone() {
        let running = HashMap::from([(10, false), (11, true), (12, false)]);
        assert_eq!(to_terminate(&running), [10, 12]);
    }

    #[test]
    fn secret_variables_reach_the_command_but_not_debug() {
        let c = Sh::cmd("echo \"$NEWPIN|$HOME\"; cat")
            .secret_env("NEWPIN", "hunter2hunter2")
            .stdin("stdinsecret");
        let out = SystemRunner.run(&c, &mut |_| {}).unwrap();
        assert_eq!(out.stdout, "hunter2hunter2|\nstdinsecret\n");
        let shown = format!("{c:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("stdinsecret"),
            "{shown}"
        );
        assert!(shown.contains("NEWPIN"));
    }

    #[test]
    fn secrets_are_not_displayed() {
        let c = Cmd::new(bin::MOKUTIL, ["--generate-hash=12345678"]).secret();
        assert!(!c.display().contains("1234"));
        assert_eq!(
            Cmd::new(bin::LSBLK, ["--json"]).display(),
            "/usr/bin/lsblk --json"
        );
    }
}
