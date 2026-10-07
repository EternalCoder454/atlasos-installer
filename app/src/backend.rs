//! The one QObject behind every page. Slow work (D-Bus, NetworkManager,
//! localectl) runs on a tokio runtime and posts back through `qt_thread()`,
//! so the GUI thread never blocks. Lists go to QML as JSON.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        /// `TELAMON_INSTALLER_DEMO` is set: nothing shown is real.
        #[qproperty(bool, demo)]
        /// Locale codes, `["de_DE.UTF-8", ...]`.
        #[qproperty(QString, languages_json, cxx_name = "languagesJson")]
        /// installer-core's keyboard::Layout list.
        #[qproperty(QString, layouts_json, cxx_name = "layoutsJson")]
        /// view::DiskRow list.
        #[qproperty(QString, disks_json, cxx_name = "disksJson")]
        /// Quick Install's choice from that list, `{"id": ..., "mode": ...}`,
        /// or "" when the user has to choose (view::quick_pick).
        #[qproperty(QString, quick_disk, cxx_name = "quickDisk")]
        /// "loading", "ready" or "error".
        #[qproperty(QString, disks_state, cxx_name = "disksState")]
        #[qproperty(QString, disks_error, cxx_name = "disksError")]
        /// installer-core's apps::catalog, in page order (read-only).
        #[qproperty(QString, apps_json, cxx_name = "appsJson")]
        /// The IDs chosen at first, a JSON array (view::default_apps).
        #[qproperty(QString, apps_default, cxx_name = "appsDefault")]
        /// network::WifiState.
        #[qproperty(QString, wifi_json, cxx_name = "wifiJson")]
        /// The first Wi-Fi read is done (the page knows whether to show).
        #[qproperty(bool, wifi_loaded, cxx_name = "wifiLoaded")]
        #[qproperty(bool, wifi_scanning, cxx_name = "wifiScanning")]
        /// The network being connected to, or "".
        #[qproperty(QString, wifi_connecting, cxx_name = "wifiConnecting")]
        #[qproperty(QString, wifi_error, cxx_name = "wifiError")]
        /// The connection made on the Wi-Fi page, for Install, or "".
        #[qproperty(QString, wifi_uuid, cxx_name = "wifiUuid")]
        #[qproperty(bool, secure_boot, cxx_name = "secureBoot")]
        #[qproperty(bool, nvidia)]
        /// This PC has a usable TPM 2.0 (from ListDisks).
        #[qproperty(bool, tpm2)]
        /// ListDisks says Secure Boot is off on this PC.
        #[qproperty(bool, secure_boot_off, cxx_name = "secureBootOff")]
        /// Demo mode: open with the start-up PIN switched on.
        #[qproperty(bool, demo_pin, cxx_name = "demoPin")]
        /// Demo mode: open with "Encrypt this disk" turned on.
        #[qproperty(bool, demo_encrypt, cxx_name = "demoEncrypt")]
        /// The smallest disk, as the Disk page says it ("40 GB").
        #[qproperty(QString, min_disk_size, cxx_name = "minDiskSize")]
        /// "idle", "running", "done" or "failed".
        #[qproperty(QString, install_state, cxx_name = "installState")]
        #[qproperty(f64, progress)]
        #[qproperty(QString, progress_text, cxx_name = "progressText")]
        #[qproperty(QString, time_left, cxx_name = "timeLeft")]
        /// The helper reported progress, so its install log exists.
        #[qproperty(bool, install_began, cxx_name = "installBegan")]
        #[qproperty(QString, install_error, cxx_name = "installError")]
        /// view::Done, after a successful install.
        #[qproperty(QString, result_json, cxx_name = "resultJson")]
        #[qproperty(bool, rebooting)]
        #[qproperty(QString, reboot_error, cxx_name = "rebootError")]
        #[namespace = "telamon_installer"]
        type Backend = super::BackendRust;

        /// Reads the languages, layouts, disks and networks.
        #[qinvokable]
        fn start(self: Pin<&mut Backend>);
        #[qinvokable]
        #[cxx_name = "refreshDisks"]
        fn refresh_disks(self: Pin<&mut Backend>);
        /// `scan`: ask NetworkManager for a fresh scan first.
        #[qinvokable]
        #[cxx_name = "refreshWifi"]
        fn refresh_wifi(self: Pin<&mut Backend>, scan: bool);
        #[qinvokable]
        #[cxx_name = "connectWifi"]
        /// `ssid_hex`: the scanned network's SSID bytes in hex (from the
        /// list); empty for a hidden network, whose typed name is used.
        /// `security` (hidden networks): "none", "wpa" or "sae".
        fn connect_wifi(
            self: Pin<&mut Backend>,
            ssid: &QString,
            ssid_hex: &QString,
            password: &QString,
            hidden: bool,
            security: &QString,
        );
        #[qinvokable]
        #[cxx_name = "clearWifiError"]
        fn clear_wifi_error(self: Pin<&mut Backend>);
        /// The layout a language suggests, e.g. `de` for de_DE.UTF-8.
        #[qinvokable]
        #[cxx_name = "defaultLayout"]
        fn default_layout(self: &Backend, locale: &QString) -> QString;
        /// Switches the live session's keyboard (live session only).
        #[qinvokable]
        #[cxx_name = "applyKeymap"]
        fn apply_keymap(self: &Backend, keymap: &QString);
        /// The Review page's line for the chosen apps (a JSON array of IDs).
        #[qinvokable]
        #[cxx_name = "reviewApps"]
        fn review_apps(self: &Backend, apps: &QString) -> QString;
        /// "none", "tpm", "tpm-pin" or "password" (view::encryption_mode).
        #[qinvokable]
        #[cxx_name = "encryptionMode"]
        fn encryption_mode(self: &Backend, tpm2: bool, on: bool, pin: bool) -> QString;
        /// The Review page's text for an encryption mode.
        #[qinvokable]
        #[cxx_name = "reviewEncryption"]
        fn review_encryption(self: &Backend, mode: &QString) -> QString;
        /// Encryption is on unless the user says otherwise, if this PC has a TPM.
        #[qinvokable]
        #[cxx_name = "encryptionDefault"]
        fn encryption_default(self: &Backend, tpm2: bool) -> bool;
        /// Whether the PIN switch starts on: with a TPM but Secure Boot off.
        #[qinvokable]
        #[cxx_name = "pinDefault"]
        fn pin_default(self: &Backend, tpm2: bool, secure_boot_off: bool) -> bool;
        /// The password (mode "password") or PIN (mode "tpm-pin") is valid
        /// and typed twice the same.
        #[qinvokable]
        #[cxx_name = "secretOk"]
        fn secret_ok(self: &Backend, mode: &QString, secret: &QString, confirm: &QString) -> bool;
        /// What is wrong with it, or "".
        #[qinvokable]
        #[cxx_name = "secretHint"]
        fn secret_hint(
            self: &Backend,
            mode: &QString,
            secret: &QString,
            confirm: &QString,
        ) -> QString;
        /// `encryption`: "none", "tpm", "tpm-pin" or "password". `password`
        /// is the password, or the PIN, for those two modes only; it is not kept.
        /// Returns whether the install was accepted (false: nothing started).
        #[qinvokable]
        fn install(
            self: Pin<&mut Backend>,
            disk_id: &QString,
            fingerprint: &QString,
            mode: &QString,
            locale: &QString,
            keymap: &QString,
            wifi_uuid: &QString,
            encryption: &QString,
            password: &QString,
            apps: &QString,
        ) -> bool;
        /// Every second while installing: the time left counts down.
        #[qinvokable]
        #[cxx_name = "tickTimeLeft"]
        fn tick_time_left(self: Pin<&mut Backend>);
        /// After a failed install: back to choosing a disk.
        #[qinvokable]
        #[cxx_name = "resetInstall"]
        fn reset_install(self: Pin<&mut Backend>);
        #[qinvokable]
        fn reboot(self: Pin<&mut Backend>);
    }

    impl cxx_qt::Threading for Backend {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn backend_make_unique() -> UniquePtr<Backend>;
    }
}

use core::pin::Pin;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;
use std::time::Instant;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use futures_util::FutureExt;
use installer_core::keyboard::{self, Layout};
use installer_core::progress::TimeLeft;

use crate::demo::{self, Flags};
use crate::{helper, network, system, view};

/// The runtime every slow call runs on.
fn rt() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("telamon-installer-rt")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// One system bus connection, shared by the helper and NetworkManager clients.
pub async fn system_bus() -> zbus::Result<zbus::Connection> {
    if let Some(c) = SYSTEM_BUS.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(c);
    }
    let c = zbus::Connection::system().await?;
    *SYSTEM_BUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(c.clone());
    Ok(c)
}

static SYSTEM_BUS: std::sync::Mutex<Option<zbus::Connection>> = std::sync::Mutex::new(None);

/// Forget the shared connection after it broke, so the next call opens a
/// new one.
pub fn reset_system_bus() {
    *SYSTEM_BUS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Runs a task's work so that a panic comes back as an error: every task
/// must post its result, or the page waits forever.
async fn guarded<T, E>(
    fut: impl Future<Output = Result<T, E>>,
    err: impl FnOnce(String) -> E,
) -> Result<T, E> {
    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(err("The installer hit an internal error.".into())))
}

fn helper_err(message: String) -> helper::Error {
    helper::Error::new(message)
}

/// How often Status is asked while following an install, and how many
/// answers in a row may be missing before the install counts as lost.
const FOLLOW_EVERY: std::time::Duration = std::time::Duration::from_secs(2);
const FOLLOW_MISSES: u32 = 30;

/// Follows an install by asking Status until the helper says it is done or
/// failed. `lost` is the error of the call that dropped, if any: it is what
/// the user gets if the helper turns out to know of no install, or can't be
/// reached for a minute.
async fn follow_status(
    flags: Option<Flags>,
    progress: impl Fn(f64, String),
    lost: Option<helper::Error>,
    encryption: &str,
) -> Result<view::Done, helper::Error> {
    let give_up = |e: helper::Error| lost.clone().unwrap_or(e);
    let mut misses = 0;
    loop {
        tokio::time::sleep(if flags.is_some() {
            std::time::Duration::from_millis(500)
        } else {
            FOLLOW_EVERY
        })
        .await;
        let r = match &flags {
            Some(f) => Ok(demo::status(f)),
            None => helper::status().await,
        };
        let r = r.and_then(|json| view::reattach(&json).map_err(helper_err));
        match r.map(view::follow) {
            Ok(view::Follow::Wait { fraction, text }) => {
                misses = 0;
                progress(fraction, text);
            }
            Ok(view::Follow::Done(d)) => return Ok(view::require_key(d, encryption)),
            Ok(view::Follow::Failed(m)) => return Err(helper::Error::new(m)),
            Ok(view::Follow::Gone) => {
                return Err(give_up(helper::Error::new(
                    "The installer service stopped before the install finished.",
                )));
            }
            Err(e) => {
                misses += 1;
                if misses >= FOLLOW_MISSES {
                    return Err(give_up(e));
                }
            }
        }
    }
}

fn q(s: &str) -> QString {
    QString::from(s)
}

pub struct BackendRust {
    demo: bool,
    languages_json: QString,
    layouts_json: QString,
    apps_json: QString,
    apps_default: QString,
    disks_json: QString,
    quick_disk: QString,
    disks_state: QString,
    disks_error: QString,
    wifi_json: QString,
    wifi_loaded: bool,
    wifi_scanning: bool,
    wifi_connecting: QString,
    wifi_error: QString,
    wifi_uuid: QString,
    secure_boot: bool,
    nvidia: bool,
    tpm2: bool,
    secure_boot_off: bool,
    demo_pin: bool,
    demo_encrypt: bool,
    min_disk_size: QString,
    install_state: QString,
    progress: f64,
    progress_text: QString,
    time_left: QString,
    install_began: bool,
    install_error: QString,
    result_json: QString,
    rebooting: bool,
    reboot_error: QString,

    flags: Option<Flags>,
    layouts: Vec<Layout>,
    started: bool,
    /// Set at the first Progress signal, so the polkit prompt doesn't count.
    install_started: Option<Instant>,
    /// Bumped by each disk refresh: an older answer that lands late is dropped.
    disks_gen: u64,
    /// A Wi-Fi refresh was asked for while one ran: run another after it.
    wifi_again: bool,
    time_left_est: TimeLeft,
    /// The network the demo is connected to.
    demo_connected: Option<String>,
}

impl Default for BackendRust {
    fn default() -> Self {
        let flags = demo::from_env();
        Self {
            demo: flags.is_some(),
            languages_json: q("[]"),
            layouts_json: q("[]"),
            apps_json: q(&serde_json::to_string(installer_core::apps::catalog())
                .unwrap_or_else(|_| "[]".into())),
            apps_default: q(
                &serde_json::to_string(&view::default_apps()).unwrap_or_else(|_| "[]".into())
            ),
            disks_json: q("[]"),
            quick_disk: QString::default(),
            disks_state: q("loading"),
            disks_error: QString::default(),
            wifi_json: q("{}"),
            wifi_loaded: false,
            wifi_scanning: false,
            wifi_connecting: QString::default(),
            wifi_error: QString::default(),
            wifi_uuid: QString::default(),
            secure_boot: false,
            nvidia: false,
            tpm2: false,
            secure_boot_off: false,
            demo_pin: false,
            demo_encrypt: false,
            min_disk_size: q(&view::min_disk_size()),
            install_state: q("idle"),
            progress: 0.0,
            progress_text: QString::default(),
            time_left: QString::default(),
            install_began: false,
            install_error: QString::default(),
            result_json: q("{}"),
            rebooting: false,
            reboot_error: QString::default(),
            flags,
            layouts: Vec::new(),
            started: false,
            install_started: None,
            disks_gen: 0,
            wifi_again: false,
            time_left_est: TimeLeft::default(),
            demo_connected: None,
        }
    }
}

impl qobject::Backend {
    pub fn start(mut self: Pin<&mut Self>) {
        if self.rust().started {
            return;
        }
        self.as_mut().rust_mut().started = true;

        let languages = system::languages();
        let layouts = system::layouts();
        let langs = serde_json::to_string(&languages).unwrap_or_else(|_| "[]".into());
        let lays = serde_json::to_string(&layouts).unwrap_or_else(|_| "[]".into());
        self.as_mut().set_languages_json(q(&langs));
        self.as_mut().set_layouts_json(q(&lays));
        self.as_mut().rust_mut().layouts = layouts;

        match self.rust().flags.clone() {
            Some(f) => {
                self.as_mut().set_secure_boot(f.mok);
                self.as_mut().set_nvidia(f.mok);
                self.as_mut().set_tpm2(!f.notpm);
                self.as_mut().set_demo_encrypt(f.encrypt);
                self.as_mut().set_demo_pin(f.pin);
            }
            None => {
                self.as_mut().set_secure_boot(system::secure_boot());
                self.as_mut().set_nvidia(system::nvidia());
            }
        }
        self.as_mut().refresh_disks();
        self.as_mut().refresh_wifi(true);
        self.as_mut().reattach();
    }

    /// A UI that starts while the helper is installing (or has installed)
    /// picks the install up where it is.
    fn reattach(self: Pin<&mut Self>) {
        let flags = self.rust().flags.clone();
        let qt = self.qt_thread();
        rt().spawn(async move {
            let r = guarded(
                async {
                    let json = match &flags {
                        Some(f) => demo::status(f),
                        // the helper may still be starting
                        None => {
                            let mut tries = 0;
                            loop {
                                match helper::status().await {
                                    Ok(j) => break j,
                                    Err(e) if tries >= 4 => return Err(e),
                                    Err(_) => {
                                        tries += 1;
                                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    }
                                }
                            }
                        }
                    };
                    view::reattach(&json).map_err(helper_err)
                },
                helper_err,
            )
            .await;
            let r = match r {
                Ok(r) => r,
                Err(e) => {
                    // No helper (a desktop, a demo): the Disk page says so
                    eprintln!(
                        "telamon-installer: couldn't ask for the install's state: {}",
                        e.message
                    );
                    return;
                }
            };
            let _ = qt.queue(move |mut obj| obj.as_mut().apply_reattach(r, flags));
        });
    }

    fn apply_reattach(mut self: Pin<&mut Self>, r: view::Reattach, flags: Option<Flags>) {
        // The user got there first (a very slow answer): leave it alone.
        if self.install_state().to_string() != "idle" {
            return;
        }
        if r != view::Reattach::Idle {
            // (the pages follow install_state, see Main.qml)
            eprintln!(
                "telamon-installer: an install is already known to the helper; opening at {}",
                r.page()
            );
        }
        match r {
            view::Reattach::Idle => {}
            view::Reattach::Running { fraction, text } => {
                self.as_mut().set_install_began(true);
                self.as_mut().set_install_error(QString::default());
                self.as_mut().set_install_state(q("running"));
                self.as_mut().on_progress(fraction, &text);
                let qt = self.qt_thread();
                rt().spawn(async move {
                    let qt_progress = qt.clone();
                    let progress = move |fraction: f64, text: String| {
                        let _ = qt_progress
                            .queue(move |mut obj| obj.as_mut().on_progress(fraction, &text));
                    };
                    // the mode is unknown here: the helper's outcome carries it
                    let r = guarded(follow_status(flags, progress, None, ""), helper_err).await;
                    let _ = qt.queue(move |mut obj| obj.as_mut().finish_install(r));
                });
            }
            view::Reattach::Done(done) => {
                self.as_mut().set_install_began(true);
                self.as_mut().finish_install(Ok(done));
            }
            view::Reattach::Failed(message) => {
                self.as_mut().set_install_began(true);
                self.as_mut()
                    .finish_install(Err(helper::Error::new(message)));
            }
        }
    }

    /// The install ended, one way or the other.
    fn finish_install(mut self: Pin<&mut Self>, r: Result<view::Done, helper::Error>) {
        match r {
            Ok(done) => {
                let json = serde_json::to_string(&done).unwrap_or_else(|_| "{}".into());
                if self.install_state().to_string() == "idle" {
                    // reattached to a finished install
                    self.as_mut().set_progress(1.0);
                } else {
                    self.as_mut().on_progress(1.0, "Finishing up");
                }
                self.as_mut().set_result_json(q(&json));
                self.as_mut().set_install_state(q("done"));
            }
            Err(e) => {
                self.as_mut().set_install_error(q(&e.message));
                self.as_mut().set_install_state(q("failed"));
            }
        }
    }

    pub fn refresh_disks(mut self: Pin<&mut Self>) {
        if self.install_state().to_string() != "idle" {
            return;
        }
        self.as_mut().set_disks_state(q("loading"));
        self.as_mut().set_disks_error(QString::default());
        let generation = self.rust().disks_gen + 1;
        self.as_mut().rust_mut().disks_gen = generation;
        let flags = self.rust().flags.clone();
        let qt = self.qt_thread();
        rt().spawn(async move {
            let r = guarded(
                async {
                    match &flags {
                        Some(f) => demo::list_disks(f).await,
                        None => helper::list_disks().await,
                    }
                },
                helper_err,
            )
            .await;
            let r = r.and_then(|json| {
                let tpm2 = view::tpm2_of(&json);
                let sb_off = view::secure_boot_off(&json);
                serde_json::from_str::<installer_core::disks::DiskList>(&json)
                    .map(|l| (view::disk_rows(&l), tpm2, sb_off))
                    .map_err(|e| {
                        helper_err(format!("The installer service gave a bad answer: {e}"))
                    })
            });
            let _ = qt.queue(move |mut obj| {
                // A newer refresh is on its way, or the install started
                // (and the helper answered Busy to this one).
                if obj.rust().disks_gen != generation || obj.install_state().to_string() != "idle" {
                    return;
                }
                obj.as_mut().apply_disks(r);
            });
        });
    }

    fn apply_disks(
        mut self: Pin<&mut Self>,
        r: Result<(Vec<view::DiskRow>, bool, bool), helper::Error>,
    ) {
        match r {
            Ok((rows, tpm2, sb_off)) => {
                self.as_mut().set_tpm2(tpm2);
                self.as_mut().set_secure_boot_off(sb_off);
                let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
                let quick = view::quick_pick(&rows)
                    .map(|(id, mode)| serde_json::json!({ "id": id, "mode": mode }).to_string())
                    .unwrap_or_default();
                self.as_mut().set_disks_json(q(&json));
                self.as_mut().set_quick_disk(q(&quick));
                self.as_mut().set_disks_state(q("ready"));
            }
            Err(e) => {
                self.as_mut().set_disks_json(q("[]"));
                self.as_mut().set_quick_disk(QString::default());
                self.as_mut().set_disks_error(q(&e.message));
                self.as_mut().set_disks_state(q("error"));
            }
        }
    }

    pub fn refresh_wifi(mut self: Pin<&mut Self>, scan: bool) {
        if *self.wifi_scanning() {
            self.as_mut().rust_mut().wifi_again = true;
            return;
        }
        self.as_mut().set_wifi_scanning(true);
        let flags = self.rust().flags.clone();
        let connected = self.rust().demo_connected.clone();
        let qt = self.qt_thread();
        rt().spawn(async move {
            let r = guarded(
                async {
                    match &flags {
                        Some(f) => {
                            if scan {
                                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                            }
                            Ok(demo::wifi(f, connected.as_deref()))
                        }
                        None => network::state(scan).await,
                    }
                },
                |e| e,
            )
            .await;
            let _ = qt.queue(move |mut obj| {
                match r {
                    Ok(st) => {
                        let json = serde_json::to_string(&st).unwrap_or_else(|_| "{}".into());
                        obj.as_mut().set_wifi_json(q(&json));
                    }
                    // A failed refresh keeps what the page shows. Without
                    // NetworkManager at the first read, the page is skipped,
                    // like having no adapter.
                    Err(e) => {
                        eprintln!("telamon-installer: couldn't read the networks: {e}");
                        if !*obj.wifi_loaded() {
                            let json = serde_json::to_string(&network::WifiState::default())
                                .unwrap_or_else(|_| "{}".into());
                            obj.as_mut().set_wifi_json(q(&json));
                        }
                    }
                }
                obj.as_mut().set_wifi_scanning(false);
                obj.as_mut().set_wifi_loaded(true);
                if std::mem::take(&mut obj.as_mut().rust_mut().wifi_again) {
                    obj.as_mut().refresh_wifi(false);
                }
            });
        });
    }

    pub fn connect_wifi(
        mut self: Pin<&mut Self>,
        ssid: &QString,
        ssid_hex: &QString,
        password: &QString,
        hidden: bool,
        security: &QString,
    ) {
        if !self.wifi_connecting().is_empty() {
            return;
        }
        let ssid = ssid.to_string();
        let password = password.to_string();
        if ssid.trim().is_empty() {
            return;
        }
        // A listed network is connected to by its bytes; a hidden one by
        // the name that was typed.
        let raw = match network::ssid_from_hex(&ssid_hex.to_string()) {
            Some(b) if !hidden => b,
            _ => ssid.as_bytes().to_vec(),
        };
        if hidden && let Err(m) = network::check_ssid_len(&ssid) {
            self.as_mut().set_wifi_error(q(&m));
            return;
        }
        let hidden_security =
            hidden.then(|| network::hidden_security(&security.to_string(), &password));
        self.as_mut().set_wifi_error(QString::default());
        // what the list compares to: the SSID's bytes, not its lossy name
        self.as_mut()
            .set_wifi_connecting(q(&network::ssid_hex(&raw)));
        let flags = self.rust().flags.clone();
        let qt = self.qt_thread();
        rt().spawn(async move {
            let r = guarded(
                async {
                    match &flags {
                        Some(_) => demo::connect(&ssid, &password).await,
                        None => network::connect(&raw, &password, hidden_security).await,
                    }
                },
                |e| e,
            )
            .await;
            drop(password);
            let _ = qt.queue(move |mut obj| {
                // The error first: the page reads it when `connecting` clears.
                match r {
                    Ok(uuid) => {
                        if obj.rust().flags.is_some() {
                            obj.as_mut().rust_mut().demo_connected = Some(ssid);
                        }
                        // The UUID NetworkManager confirmed is what Install
                        // copies, whatever a refresh still in flight says.
                        obj.as_mut().set_wifi_uuid(q(&uuid));
                        obj.as_mut().refresh_wifi(false);
                    }
                    Err(e) => obj.as_mut().set_wifi_error(q(&e)),
                }
                obj.as_mut().set_wifi_connecting(QString::default());
            });
        });
    }

    pub fn clear_wifi_error(self: Pin<&mut Self>) {
        self.set_wifi_error(QString::default());
    }

    pub fn default_layout(&self, locale: &QString) -> QString {
        q(&keyboard::default_layout(
            &locale.to_string(),
            &self.rust().layouts,
        ))
    }

    pub fn apply_keymap(&self, keymap: &QString) {
        if self.rust().flags.is_some() {
            return;
        }
        let k = keymap.to_string();
        rt().spawn(async move {
            if let Err(e) = system::apply_keymap(&k).await {
                eprintln!("telamon-installer: couldn't switch the keyboard to {k}: {e}");
            }
        });
    }

    pub fn encryption_mode(&self, tpm2: bool, on: bool, pin: bool) -> QString {
        q(view::encryption_mode(tpm2, on, pin))
    }

    pub fn review_encryption(&self, mode: &QString) -> QString {
        q(view::review_encryption(&mode.to_string()))
    }

    pub fn encryption_default(&self, tpm2: bool) -> bool {
        view::encryption_default(tpm2)
    }

    pub fn pin_default(&self, tpm2: bool, secure_boot_off: bool) -> bool {
        view::pin_default(tpm2, secure_boot_off)
    }

    pub fn secret_ok(&self, mode: &QString, secret: &QString, confirm: &QString) -> bool {
        view::secret_ok(&mode.to_string(), &secret.to_string(), &confirm.to_string())
    }

    pub fn secret_hint(&self, mode: &QString, secret: &QString, confirm: &QString) -> QString {
        q(&view::secret_hint(
            &mode.to_string(),
            &secret.to_string(),
            &confirm.to_string(),
        ))
    }

    pub fn review_apps(&self, apps: &QString) -> QString {
        q(&view::apps_summary(&apps.to_string()))
    }

    pub fn install(
        mut self: Pin<&mut Self>,
        disk_id: &QString,
        fingerprint: &QString,
        mode: &QString,
        locale: &QString,
        keymap: &QString,
        wifi_uuid: &QString,
        encryption: &QString,
        password: &QString,
        apps: &QString,
    ) -> bool {
        if self.install_state().to_string() != "idle" || *self.rebooting() {
            return false;
        }
        let args = [
            disk_id,
            fingerprint,
            mode,
            locale,
            keymap,
            wifi_uuid,
            encryption,
            password,
        ]
        .map(|s| s.to_string());
        // QML checks these too; the helper checks them again.
        if args[..5].iter().any(|a| a.is_empty())
            || !matches!(args[2].as_str(), "erase" | "free-space")
        {
            eprintln!(
                "telamon-installer: refusing to install with {:?}",
                &args[..5]
            );
            return false;
        }
        // The password or PIN goes to the helper for those modes only, and
        // only if it is one the helper accepts. Never printed.
        let password_ok = match args[6].as_str() {
            "none" | "tpm" => args[7].is_empty(),
            "password" => view::password_problem(&args[7]).is_none(),
            "tpm-pin" => view::pin_problem(&args[7]).is_none(),
            _ => false,
        };
        if !password_ok {
            eprintln!(
                "telamon-installer: refusing to install with encryption {:?}",
                args[6]
            );
            return false;
        }
        let apps = match view::parse_apps(&apps.to_string()) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("telamon-installer: refusing to install: {e}");
                return false;
            }
        };
        self.as_mut().set_install_error(QString::default());
        self.as_mut().set_install_began(false);
        self.as_mut().set_progress(0.0);
        self.as_mut().set_progress_text(q("Getting ready"));
        {
            let mut r = self.as_mut().rust_mut();
            r.install_started = None;
            r.time_left_est = TimeLeft::default();
        }
        let left = self
            .as_mut()
            .rust_mut()
            .time_left_est
            .text(0.0, Default::default());
        self.as_mut().set_time_left(q(&left));
        self.as_mut().set_install_state(q("running"));

        let encryption = args[6].clone();
        let flags = self.rust().flags.clone();
        let qt = self.qt_thread();
        let qt_progress = qt.clone();
        rt().spawn(async move {
            let qt_follow = qt_progress.clone();
            let progress = move |fraction: f64, text: String| {
                let _ = qt_progress.queue(move |mut obj| obj.as_mut().on_progress(fraction, &text));
            };
            let r = guarded(
                async {
                    let r = match &flags {
                        Some(f) => demo::install(f, progress).await,
                        None => helper::install(args, apps, progress).await,
                    };
                    match r {
                        // Install succeeded even if its answer can't be
                        // read: never offer to install again over a
                        // finished system.
                        Ok(json) => Ok(match view::done_from_outcome(&json) {
                            Ok(d) => view::require_key(d, &encryption),
                            // The answer is unreadable, but Status gives an
                            // authorised caller the same outcome, key included.
                            Err(m) => {
                                let st = match &flags {
                                    Some(f) => Ok(demo::status(f)),
                                    None => helper::status().await,
                                };
                                match st.ok().and_then(|j| view::reattach(&j).ok()) {
                                    Some(view::Reattach::Done(d)) => {
                                        view::require_key(d, &encryption)
                                    }
                                    _ => view::require_key(
                                        view::Done {
                                            warnings: vec![format!(
                                                "The installer's report couldn't be read ({m})."
                                            )],
                                            ..Default::default()
                                        },
                                        &encryption,
                                    ),
                                }
                            }
                        }),
                        // The call was lost, not refused: the install may
                        // be going on. Ask the helper until it says.
                        // Busy: the helper is already installing (an
                        // earlier run of this UI started it).
                        Err(e) if e.transient || e.busy => {
                            let progress = move |fraction: f64, text: String| {
                                let _ = qt_follow.queue(move |mut obj| {
                                    obj.as_mut().on_progress(fraction, &text)
                                });
                            };
                            follow_status(flags.clone(), progress, Some(e), &encryption).await
                        }
                        Err(e) => Err(e),
                    }
                },
                helper_err,
            )
            .await;
            let _ = qt.queue(move |mut obj| obj.as_mut().finish_install(r));
        });
        true
    }

    fn on_progress(mut self: Pin<&mut Self>, fraction: f64, text: &str) {
        if self.install_state().to_string() != "running" {
            return;
        }
        // The helper never goes backwards; neither does the bar.
        let fraction = fraction.clamp(0.0, 1.0).max(*self.progress());
        let started = *self
            .as_mut()
            .rust_mut()
            .install_started
            .get_or_insert_with(Instant::now);
        let elapsed = started.elapsed();
        let left = self
            .as_mut()
            .rust_mut()
            .time_left_est
            .text(fraction, elapsed);
        self.as_mut().set_install_began(true);
        self.as_mut().set_progress(fraction);
        if !text.is_empty() {
            self.as_mut().set_progress_text(q(text));
        }
        self.as_mut().set_time_left(q(&left));
    }

    pub fn tick_time_left(mut self: Pin<&mut Self>) {
        if self.install_state().to_string() != "running" {
            return;
        }
        let Some(started) = self.rust().install_started else {
            return;
        };
        let left = self
            .as_mut()
            .rust_mut()
            .time_left_est
            .tick(started.elapsed());
        if *self.time_left() != q(&left) {
            self.as_mut().set_time_left(q(&left));
        }
    }

    pub fn reset_install(mut self: Pin<&mut Self>) {
        if self.install_state().to_string() != "failed" {
            return;
        }
        self.as_mut().set_install_error(QString::default());
        self.as_mut().set_progress(0.0);
        self.as_mut().set_install_state(q("idle"));
    }

    pub fn reboot(mut self: Pin<&mut Self>) {
        if *self.rebooting() || self.install_state().to_string() == "running" {
            return;
        }
        self.as_mut().set_reboot_error(QString::default());
        self.as_mut().set_rebooting(true);
        let demo = self.rust().flags.is_some();
        let qt = self.qt_thread();
        rt().spawn(async move {
            let r = if demo {
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                Ok(())
            } else {
                helper::reboot().await
            };
            if let Err(e) = r {
                let _ = qt.queue(move |mut obj| {
                    obj.as_mut().set_reboot_error(q(&e.message));
                    obj.as_mut().set_rebooting(false);
                });
            }
        });
    }
}
