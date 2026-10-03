//! D-Bus glue: the `InstallerHelper1` interface, polkit checks, one install
//! at a time, and the idle exit. Modelled on Atlas Updater's
//! atlas-system-helper.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Value;

use crate::install::{self, Env, Request};
use crate::run::{self, Cmd, Runner, bin, lock};

pub const BUS_NAME: &str = "net.eterneon.atlas.InstallerHelper";
pub const OBJECT_PATH: &str = "/net/eterneon/atlas/InstallerHelper";

/// Exit after this long with no calls.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

const POLKIT_ALLOW_USER_INTERACTION: u32 = 1;

pub mod action {
    pub const LIST_DISKS: &str = "net.eterneon.atlas.installer.list-disks";
    pub const INSTALL: &str = "net.eterneon.atlas.installer.install";
    pub const REBOOT: &str = "net.eterneon.atlas.installer.reboot";
}

/// Errors returned over D-Bus as `net.eterneon.atlas.Error.*` (the same
/// names as Atlas Updater's helper).
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "net.eterneon.atlas.Error")]
pub enum HelperError {
    #[zbus(error)]
    ZBus(zbus::Error),
    InvalidArgument(String),
    NotAuthorized(String),
    Busy(String),
    /// The helper is exiting; the client retries and D-Bus starts a new one.
    ShuttingDown(String),
    Failed(String),
}

#[zbus::proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority",
    gen_blocking = false
)]
trait Authority {
    #[allow(clippy::type_complexity)]
    fn check_authorization(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        action_id: &str,
        details: &HashMap<&str, &str>,
        flags: u32,
        cancellation_id: &str,
    ) -> zbus::Result<(bool, bool, HashMap<String, String>)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_blocking = false
)]
trait Login {
    fn inhibit(
        &self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedFd>;
}

/// Unlike an update, an install stopped half-way leaves a disk that boots
/// nothing, so shutdown and sleep wait while one runs. Held until dropped;
/// without logind the install goes ahead anyway.
async fn block_shutdown(conn: &zbus::Connection) -> Option<zbus::zvariant::OwnedFd> {
    let login = LoginProxy::new(conn).await.ok()?;
    match login
        .inhibit(
            "shutdown:sleep:idle",
            "AtlasOS Installer",
            "AtlasOS is being installed",
            "block",
        )
        .await
    {
        Ok(fd) => Some(fd),
        Err(e) => {
            eprintln!("cannot block shutdown during the install: {e}");
            None
        }
    }
}

#[derive(Default)]
struct State {
    active: usize,
    last: Option<Instant>,
    closing: bool,
}

/// Calls in flight and the time of the last one. One lock, so a call cannot
/// slip in between the idle check and the decision to exit.
#[derive(Default)]
pub struct Activity(Mutex<State>);

pub struct ActivityGuard(Arc<Activity>);

impl Activity {
    /// Register a call; `None` once the helper is shutting down.
    pub fn enter(self: &Arc<Self>) -> Option<ActivityGuard> {
        let mut st = lock(&self.0);
        if st.closing {
            return None;
        }
        st.active += 1;
        Some(ActivityGuard(self.clone()))
    }

    /// If nothing is in flight and the last call ended `timeout` ago, mark
    /// the helper as closing and return true.
    pub fn close_if_idle(&self, timeout: Duration, started: Instant) -> bool {
        let mut st = lock(&self.0);
        if st.active == 0 && st.last.unwrap_or(started).elapsed() >= timeout {
            st.closing = true;
        }
        st.closing
    }

    pub fn begin_close(&self) {
        lock(&self.0).closing = true;
    }

    /// Wait until no call is in flight; false if `limit` ran out first.
    pub async fn wait_drained(&self, limit: Duration) -> bool {
        let end = Instant::now() + limit;
        while lock(&self.0).active > 0 {
            if Instant::now() >= end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        true
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut st = lock(&self.0.0);
        st.active = st.active.saturating_sub(1);
        st.last = Some(Instant::now());
    }
}

struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub struct Service {
    runner: Arc<dyn Runner>,
    env: Env,
    activity: Arc<Activity>,
    /// An install is running, or the computer is restarting. Every call
    /// that touches the disks is refused meanwhile.
    busy: Arc<AtomicBool>,
    /// Held while the disks are read or written: listings wait for each
    /// other, and an install waits for a listing in flight (ListDisks
    /// mounts EFI partitions to look at them).
    disks: Arc<tokio::sync::Mutex<()>>,
}

const SHUTTING_DOWN: &str = "the helper is shutting down, try again";
const BUSY: &str = "AtlasOS is being installed, or the computer is restarting";

impl Service {
    pub fn new(runner: Arc<dyn Runner>, env: Env) -> Service {
        Service {
            runner,
            env,
            activity: Arc::new(Activity::default()),
            busy: Arc::new(AtomicBool::new(false)),
            disks: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    fn begin(&self) -> Result<ActivityGuard, HelperError> {
        self.activity
            .enter()
            .ok_or_else(|| HelperError::ShuttingDown(SHUTTING_DOWN.into()))
    }

    fn refuse_if_busy(&self) -> Result<(), HelperError> {
        if self.busy.load(Ordering::Acquire) {
            Err(HelperError::Busy(BUSY.into()))
        } else {
            Ok(())
        }
    }

    fn take_busy(&self) -> Result<BusyGuard, HelperError> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| BusyGuard(self.busy.clone()))
            .map_err(|_| HelperError::Busy(BUSY.into()))
    }

    /// The work behind ListDisks, without polkit (tests).
    pub async fn do_list_disks(&self) -> Result<String, HelperError> {
        let guard = self.begin()?;
        self.refuse_if_busy()?;
        let disks = self.disks.clone().lock_owned().await;
        self.refuse_if_busy()?;
        let (runner, env) = (self.runner.clone(), self.env.clone());
        tokio::task::spawn_blocking(move || {
            let _keep = (guard, disks);
            install::list_disks(runner.as_ref(), &env)
        })
        .await
        .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
        .map_err(HelperError::Failed)
        .and_then(|l| serde_json::to_string(&l).map_err(|e| HelperError::Failed(e.to_string())))
    }

    /// The work behind Install, without polkit (tests). `progress` gets
    /// (step, fraction, text). `inhibit` is awaited once the install may go
    /// ahead (a logind lock, see [`block_shutdown`]). The guards and the
    /// lock move into the worker, so a caller that goes away does not stop
    /// the install, let the helper exit or allow a shutdown.
    pub async fn do_install<F, I>(
        &self,
        req: Request,
        inhibit: I,
        mut progress: F,
    ) -> Result<String, HelperError>
    where
        F: FnMut(&'static str, f64, &'static str) + Send,
        I: std::future::Future<Output = Option<zbus::zvariant::OwnedFd>>,
    {
        let guard = self.begin()?;
        let busy = self.take_busy()?;
        let disks = self.disks.clone().lock_owned().await;
        let inhibit = inhibit.await;
        let (runner, env) = (self.runner.clone(), self.env.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut work = tokio::task::spawn_blocking(move || {
            let _keep = (guard, busy, disks, inhibit);
            install::install(runner.as_ref(), &env, &req, &mut |p| {
                let _ = tx.send((p.stage.id(), p.fraction, p.text()));
            })
        });
        let result = loop {
            tokio::select! {
                Some((step, fraction, text)) = rx.recv() => progress(step, fraction, text),
                r = &mut work => break r,
            }
        };
        while let Ok((step, fraction, text)) = rx.try_recv() {
            progress(step, fraction, text);
        }
        let outcome = result
            .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
            .map_err(HelperError::Failed)?;
        serde_json::to_string(&outcome).map_err(|e| HelperError::Failed(e.to_string()))
    }

    async fn authorize(
        &self,
        header: &Header<'_>,
        conn: &zbus::Connection,
        action: &str,
    ) -> Result<(), HelperError> {
        let denied = |m: String| HelperError::NotAuthorized(m);
        let sender = header
            .sender()
            .ok_or_else(|| denied("call has no sender".into()))?
            .to_string();
        let authority = AuthorityProxy::new(conn)
            .await
            .map_err(|e| denied(format!("cannot reach polkit: {e}")))?;
        let subject = (
            "system-bus-name",
            HashMap::from([("name", Value::from(sender.as_str()))]),
        );
        let (authorized, _challenge, _details) = authority
            .check_authorization(
                &subject,
                action,
                &HashMap::new(),
                POLKIT_ALLOW_USER_INTERACTION,
                "",
            )
            .await
            .map_err(|e| denied(format!("polkit check failed: {e}")))?;
        if authorized {
            Ok(())
        } else {
            Err(denied(format!("not authorized for {action}")))
        }
    }
}

#[zbus::interface(name = "net.eterneon.atlas.InstallerHelper1")]
impl Service {
    /// JSON: `{"disks": [...], "hidden": [...]}` (installer-core's DiskList).
    async fn list_disks(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<String, HelperError> {
        // count the call while polkit is asked, so we do not exit under it
        let _guard = self.begin()?;
        self.authorize(&header, conn, action::LIST_DISKS).await?;
        self.do_list_disks().await
    }

    /// Install to the disk `disk_id` (a kernel name from ListDisks), refused
    /// unless it still has the `fingerprint` ListDisks gave.
    /// `mode`: `erase` or `free-space`. `keymap`: `de` or `de(nodeadkeys)`.
    /// `wifi_uuid`: a NetworkManager connection to carry over, or empty.
    /// Emits Progress while it runs; returns the Outcome as JSON.
    #[allow(clippy::too_many_arguments)]
    async fn install(
        &self,
        disk_id: String,
        fingerprint: String,
        mode: String,
        locale: String,
        keymap: String,
        wifi_uuid: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<String, HelperError> {
        let _guard = self.begin()?;
        self.authorize(&header, conn, action::INSTALL).await?;
        if fingerprint.is_empty() {
            return Err(HelperError::InvalidArgument(
                "the disk's fingerprint from ListDisks is required".into(),
            ));
        }
        let req = Request::new(&disk_id, &fingerprint, &mode, &locale, &keymap, &wifi_uuid)
            .map_err(HelperError::InvalidArgument)?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let work = self.do_install(req, block_shutdown(conn), move |s, f, t| {
            let _ = tx.send((s, f, t));
        });
        tokio::pin!(work);
        let result = loop {
            tokio::select! {
                Some((step, fraction, text)) = rx.recv() => {
                    let _ = Service::progress(&emitter, step, fraction, text).await;
                }
                r = &mut work => break r,
            }
        };
        while let Ok((step, fraction, text)) = rx.try_recv() {
            let _ = Service::progress(&emitter, step, fraction, text).await;
        }
        result
    }

    /// Restart the computer (refused while installing).
    async fn reboot(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> Result<(), HelperError> {
        let _guard = self.begin()?;
        self.authorize(&header, conn, action::REBOOT).await?;
        let busy = self.take_busy()?;
        let runner = self.runner.clone();
        tokio::task::spawn_blocking(move || {
            run::run(runner.as_ref(), Cmd::new(bin::SYSTEMCTL, ["reboot"]))
        })
        .await
        .map_err(|e| HelperError::Failed(format!("worker failed: {e}")))?
        .map_err(HelperError::Failed)?;
        // the computer is going down: nothing may touch the disks now
        std::mem::forget(busy);
        Ok(())
    }

    /// `step`: prepare, partition, format, copy, bootloader, settings or
    /// finish. `fraction`: of the whole install, 0 to 1. `text`: for display.
    #[zbus(signal)]
    async fn progress(
        emitter: &SignalEmitter<'_>,
        step: &str,
        fraction: f64,
        text: &str,
    ) -> zbus::Result<()>;
}

/// How long a running install may go on after SIGTERM before its commands
/// are asked to stop (the unit gives the helper 60 s in all).
const TERM_GRACE: Duration = Duration::from_secs(40);

/// Serve until idle for `idle_timeout` or SIGTERM. Either way: refuse new
/// calls, release the bus name, then wait for calls in flight.
pub async fn serve(
    builder: zbus::connection::Builder<'static>,
    service: Service,
    idle_timeout: Duration,
) -> zbus::Result<()> {
    let activity = service.activity.clone();
    let started = Instant::now();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let conn = builder
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()
        .await?;
    let tick = (idle_timeout / 4).max(Duration::from_millis(50));
    let terminated = loop {
        tokio::select! {
            _ = term.recv() => break true,
            _ = tokio::time::sleep(tick) => {
                if activity.close_if_idle(idle_timeout, started) {
                    break false;
                }
            }
        }
    };
    activity.begin_close();
    let _ = conn.release_name(BUS_NAME).await;
    let limit = if terminated {
        TERM_GRACE
    } else {
        Duration::from_secs(2 * 3600)
    };
    if !activity.wait_drained(limit).await {
        run::terminate_running();
        activity.wait_drained(Duration::from_secs(10)).await;
    }
    // A call's guard goes before zbus sends its reply: give the last reply
    // and signals time to leave before the connection closes.
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(conn);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_only_when_idle_and_then_refuses_calls() {
        let a = Arc::new(Activity::default());
        let start = Instant::now();
        assert!(!a.close_if_idle(Duration::from_secs(60), start));
        let g = a.enter().unwrap();
        assert!(!a.close_if_idle(Duration::ZERO, start), "call in flight");
        drop(g);
        assert!(a.close_if_idle(Duration::ZERO, start));
        assert!(a.enter().is_none(), "refused once closing");
    }

    struct Nothing;
    impl Runner for Nothing {
        fn run(&self, _: &Cmd, _: &mut dyn FnMut(Option<&str>)) -> Result<run::Output, String> {
            Err("nothing runs here".into())
        }
    }

    #[tokio::test]
    async fn everything_waits_for_a_running_install() {
        let s = Service::new(Arc::new(Nothing), Env::system());
        s.busy.store(true, Ordering::Release);
        assert!(matches!(s.do_list_disks().await, Err(HelperError::Busy(_))));
        let req = Request::new("sda", "", "erase", "en_US.UTF-8", "us", "").unwrap();
        assert!(matches!(
            s.do_install(req.clone(), async { None }, |_, _, _| {})
                .await,
            Err(HelperError::Busy(_))
        ));
        s.busy.store(false, Ordering::Release);
        // free again: the call runs (and fails, as nothing can run here)
        assert!(
            matches!(s.do_list_disks().await, Err(HelperError::Failed(m)) if m.contains("nothing runs"))
        );
        assert!(!s.busy.load(Ordering::Acquire), "released after the call");
    }

    struct Slow;
    impl Runner for Slow {
        fn run(&self, _: &Cmd, _: &mut dyn FnMut(Option<&str>)) -> Result<run::Output, String> {
            std::thread::sleep(Duration::from_millis(300));
            Err("slow and failing".into())
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listings_and_installs_wait_for_each_other_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env {
            root: dir.path().join("root"),
            target: dir.path().join("target"),
            run_dir: dir.path().join("run"),
        };
        let s = Arc::new(Service::new(Arc::new(Slow), env));
        let req = Request::new("sda", "", "erase", "en_US.UTF-8", "us", "").unwrap();
        let (a, b, c) = tokio::join!(
            s.do_list_disks(),
            s.do_list_disks(),
            s.do_install(req, async { None }, |_, _, _| {})
        );
        // the first listing runs; the install waits for it rather than
        // failing; a listing queued behind the install is then refused
        let slow = |r: &Result<String, HelperError>| matches!(r, Err(HelperError::Failed(m)) if m.contains("slow"));
        assert!(slow(&a), "{a:?}");
        assert!(slow(&c), "{c:?}");
        assert!(matches!(b, Err(HelperError::Busy(_))), "{b:?}");
    }
}
