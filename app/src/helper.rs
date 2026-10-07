//! Client for `net.eterneon.telamon.InstallerHelper1` (docs/helper-api.md).
//! Async, on the backend's tokio runtime. There is no method timeout:
//! Install runs for minutes.

use futures_util::StreamExt;

#[zbus::proxy(
    interface = "net.eterneon.telamon.InstallerHelper1",
    default_service = "net.eterneon.telamon.InstallerHelper",
    default_path = "/net/eterneon/telamon/InstallerHelper",
    gen_blocking = false
)]
pub trait InstallerHelper1 {
    fn list_disks(&self) -> zbus::Result<String>;
    fn install(
        &self,
        disk_id: &str,
        fingerprint: &str,
        mode: &str,
        locale: &str,
        keymap: &str,
        wifi_uuid: &str,
        encryption: &str,
        password: &str,
        apps: &[String],
    ) -> zbus::Result<String>;
    fn status(&self) -> zbus::Result<String>;
    fn reboot(&self) -> zbus::Result<()>;
    #[zbus(signal)]
    fn progress(&self, step: &str, fraction: f64, text: &str) -> zbus::Result<()>;
}

/// What the helper (or the bus) said, in words for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub busy: bool,
    /// The call didn't get an answer (the connection dropped, the bus
    /// timed out), so the helper may be fine and still at work: ask Status.
    pub transient: bool,
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error {
            busy: false,
            transient: false,
            message: message.into(),
        }
    }

    fn lost(message: impl Into<String>) -> Error {
        Error {
            transient: true,
            ..Error::new(message)
        }
    }
}

const PREFIX: &str = "net.eterneon.telamon.Error.";

/// Maps an error name and message to what the UI says.
pub fn describe(name: &str, message: Option<&str>) -> Error {
    let msg = message.unwrap_or("").trim();
    match name.strip_prefix(PREFIX) {
        Some("Busy") => Error {
            busy: true,
            transient: false,
            message: "Telamon OS is already being installed, or the computer is restarting.".into(),
        },
        Some("NotAuthorized") => Error::new("The installer wasn't allowed to change the disks."),
        Some("ShuttingDown") => Error::new("The installer service stopped. Try again."),
        Some("InvalidArgument") => {
            Error::new(format!("The installer service refused the request: {msg}"))
        }
        Some(_) if !msg.is_empty() => Error::new(capitalize(msg)),
        Some(_) => Error::new("Something went wrong."),
        None => match name {
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.DBus.Error.Spawn.ChildExited"
            | "org.freedesktop.DBus.Error.Spawn.Failed"
            | "org.freedesktop.systemd1.NoSuchUnit" => Error::new(
                "The installer service isn't available. Restart the computer from the installer media and try again.",
            ),
            "org.freedesktop.DBus.Error.NoReply"
            | "org.freedesktop.DBus.Error.Disconnected"
            | "org.freedesktop.DBus.Error.TimedOut"
            | "org.freedesktop.DBus.Error.NoServer" => Error::lost(format!(
                "The installer service couldn't be reached ({name})."
            )),
            _ => Error::new(format!(
                "The installer service couldn't be reached ({name})."
            )),
        },
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn map(e: zbus::Error) -> Error {
    match e {
        zbus::Error::MethodError(name, msg, _) => describe(name.as_str(), msg.as_deref()),
        zbus::Error::FDO(fdo) => {
            let name = fdo_name(&fdo);
            let e = describe(&name, Some(&fdo.to_string()));
            if e.transient {
                crate::backend::reset_system_bus();
            }
            e
        }
        // Input/output and the like: the connection itself broke.
        other => {
            crate::backend::reset_system_bus();
            Error::lost(format!(
                "The installer service couldn't be reached: {other}"
            ))
        }
    }
}

fn fdo_name(e: &zbus::fdo::Error) -> String {
    use zbus::DBusError;
    e.name().to_string()
}

fn shutting_down(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::MethodError(n, _, _) if n.as_str() == "net.eterneon.telamon.Error.ShuttingDown")
}

async fn proxy() -> Result<InstallerHelper1Proxy<'static>, Error> {
    let conn = crate::backend::system_bus()
        .await
        .map_err(|e| Error::new(format!("The system bus isn't available: {e}")))?;
    InstallerHelper1Proxy::new(&conn).await.map_err(map)
}

/// The helper was exiting (idle) as the call came in: one more try starts it again.
macro_rules! retry_once {
    ($call:expr) => {{
        match $call.await {
            Err(e) if shutting_down(&e) => $call.await,
            r => r,
        }
    }};
}

pub async fn list_disks() -> Result<String, Error> {
    let p = proxy().await?;
    retry_once!(p.list_disks()).map_err(map)
}

/// Runs the install, handing each Progress signal to `progress`.
pub async fn install(
    args: [String; 8],
    apps: Vec<String>,
    progress: impl Fn(f64, String) + Send + 'static,
) -> Result<String, Error> {
    let p = proxy().await?;
    // Subscribe first, so no early signal is missed.
    let mut stream = p.receive_progress().await.map_err(map)?;
    let forward = tokio::spawn(async move {
        while let Some(sig) = stream.next().await {
            if let Ok(a) = sig.args() {
                progress(a.fraction, a.text.to_string());
            }
        }
    });
    let [disk, fp, mode, locale, keymap, wifi, encryption, password] = &args;
    let r = retry_once!(p.install(
        disk, fp, mode, locale, keymap, wifi, encryption, password, &apps
    ))
    .map_err(map);
    drop(args);
    forward.abort();
    r
}

/// The install's state (see `view::reattach`).
pub async fn status() -> Result<String, Error> {
    let p = proxy().await?;
    retry_once!(p.status()).map_err(map)
}

pub async fn reboot() -> Result<(), Error> {
    let p = proxy().await?;
    retry_once!(p.reboot()).map_err(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_texts() {
        let e = describe("net.eterneon.telamon.Error.Busy", Some("x"));
        assert!(e.busy);
        assert_eq!(
            describe(
                "net.eterneon.telamon.Error.Failed",
                Some("the disk changed since it was listed")
            )
            .message,
            "The disk changed since it was listed"
        );
        assert_eq!(
            describe("net.eterneon.telamon.Error.Failed", None).message,
            "Something went wrong."
        );
        assert!(
            describe("org.freedesktop.DBus.Error.ServiceUnknown", None)
                .message
                .contains("isn't available")
        );
        assert!(
            describe("org.freedesktop.DBus.Error.AccessDenied", None)
                .message
                .contains("AccessDenied")
        );
        assert!(!describe("net.eterneon.telamon.Error.NotAuthorized", None).busy);
        assert!(!describe("net.eterneon.telamon.Error.Failed", Some("x")).transient);
    }

    #[test]
    fn a_call_without_an_answer_is_transient() {
        assert!(describe("org.freedesktop.DBus.Error.NoReply", None).transient);
        assert!(describe("org.freedesktop.DBus.Error.Disconnected", None).transient);
        assert!(!describe("org.freedesktop.DBus.Error.AccessDenied", None).transient);
        assert!(!describe("net.eterneon.telamon.Error.Busy", None).transient);
    }
}
