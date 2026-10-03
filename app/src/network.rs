//! Wi-Fi through NetworkManager on the system bus. The connection the user
//! makes is saved system-wide, with its password in the keyfile, so the
//! helper can copy it into the installed system (`wifi_uuid`).

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager",
    gen_blocking = false
)]
trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager",
    gen_blocking = false
)]
trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;
    /// (new state, old state, reason)
    #[zbus(signal, name = "StateChanged")]
    fn device_state_changed(&self, new_state: u32, old_state: u32, reason: u32)
    -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager",
    gen_blocking = false
)]
trait Wireless {
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager",
    gen_blocking = false
)]
trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager",
    gen_blocking = false
)]
trait Active {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn uuid(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager",
    gen_blocking = false
)]
trait SettingsConnection {
    fn delete(&self) -> zbus::Result<()>;
}

const DEVICE_ETHERNET: u32 = 1;
const DEVICE_WIFI: u32 = 2;
const DEVICE_ACTIVATED: u32 = 100;
const ACTIVE_ACTIVATED: u32 = 2;
const ACTIVE_DEACTIVATED: u32 = 4;
const DEVICE_DISCONNECTED: u32 = 30;
const DEVICE_IP_CONFIG: u32 = 70;
const DEVICE_FAILED: u32 = 120;

/// How a network is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    Open,
    Owe,
    Wep,
    Psk,
    Sae,
    /// WPA Enterprise (802.1X): not offered.
    Enterprise,
}

impl Security {
    /// From an access point's Flags, WpaFlags and RsnFlags.
    pub fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Security {
        const PRIVACY: u32 = 0x1;
        const PSK: u32 = 0x100;
        const EAP: u32 = 0x200;
        const SAE: u32 = 0x400;
        const OWE: u32 = 0x800;
        const OWE_TM: u32 = 0x1000;
        const EAP_192: u32 = 0x2000;
        let all = wpa | rsn;
        if all & (EAP | EAP_192) != 0 && all & (PSK | SAE) == 0 {
            Security::Enterprise
        } else if all & PSK != 0 {
            // WPA2, and WPA2/WPA3 transition networks
            Security::Psk
        } else if all & SAE != 0 {
            Security::Sae
        } else if all & (OWE | OWE_TM) != 0 {
            Security::Owe
        } else if flags & PRIVACY != 0 {
            Security::Wep
        } else {
            Security::Open
        }
    }

    pub fn needs_password(self) -> bool {
        matches!(self, Security::Wep | Security::Psk | Security::Sae)
    }
}

/// An SSID is up to 32 raw bytes, not necessarily text: the bytes go to
/// NetworkManager as they came, and the name shown is a lossy reading.
pub fn ssid_hex(raw: &[u8]) -> String {
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

/// The bytes behind [`ssid_hex`]; `None` if it isn't one.
pub fn ssid_from_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.is_empty() || !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    /// For display (invalid UTF-8 shown as replacement characters).
    pub ssid: String,
    /// The SSID's bytes in hex: what identifies the network, and what
    /// `connect` takes.
    pub ssid_hex: String,
    /// 0 to 100.
    pub strength: u8,
    pub security: Security,
    pub active: bool,
}

/// What the Wi-Fi page shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiState {
    /// There is a Wi-Fi adapter.
    pub available: bool,
    /// Connected by cable: the page is skipped.
    pub wired: bool,
    pub networks: Vec<Network>,
    pub active_ssid: String,
    /// The active Wi-Fi connection, which Install carries over.
    pub active_uuid: String,
}

/// One entry per name, the strongest first, the connected one on top.
/// Hidden networks (no name) are left out.
pub fn merge(aps: Vec<Network>) -> Vec<Network> {
    let mut out: Vec<Network> = Vec::new();
    for ap in aps {
        let blank = ssid_from_hex(&ap.ssid_hex).is_none_or(|b| b.iter().all(|c| *c == 0));
        if blank || ap.ssid.trim().is_empty() {
            continue;
        }
        match out.iter_mut().find(|n| n.ssid_hex == ap.ssid_hex) {
            Some(n) => {
                n.active |= ap.active;
                if ap.strength > n.strength {
                    n.strength = ap.strength;
                    n.security = ap.security;
                }
            }
            None => out.push(ap),
        }
    }
    out.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.strength.cmp(&a.strength))
            .then_with(|| a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    out
}

/// A random UUID (version 4) for a new connection.
pub fn new_uuid() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| format!("No random numbers: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        h[0..4].concat(),
        h[4..6].concat(),
        h[6..8].concat(),
        h[8..10].concat(),
        h[10..16].concat()
    ))
}

fn err(e: zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(m), _) => m,
        other => other.to_string(),
    }
}

async fn conn() -> Result<zbus::Connection, String> {
    crate::backend::system_bus()
        .await
        .map_err(|e| e.to_string())
}

async fn device(c: &zbus::Connection, p: &OwnedObjectPath) -> zbus::Result<DeviceProxy<'static>> {
    DeviceProxy::builder(c)
        .path(p.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// The first Wi-Fi device, and whether a cable is connected.
async fn devices(c: &zbus::Connection) -> Result<(Option<OwnedObjectPath>, bool), String> {
    let nm = NetworkManagerProxy::new(c).await.map_err(err)?;
    let mut wifi = None;
    let mut wired = false;
    for p in nm.get_devices().await.map_err(err)? {
        let d = device(c, &p).await.map_err(err)?;
        match d.device_type().await.map_err(err)? {
            DEVICE_WIFI if wifi.is_none() => wifi = Some(p),
            DEVICE_ETHERNET => wired |= d.state().await.map_err(err)? == DEVICE_ACTIVATED,
            _ => {}
        }
    }
    Ok((wifi, wired))
}

async fn wireless(
    c: &zbus::Connection,
    p: &OwnedObjectPath,
) -> zbus::Result<WirelessProxy<'static>> {
    WirelessProxy::builder(c)
        .path(p.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

async fn active(c: &zbus::Connection, p: &OwnedObjectPath) -> zbus::Result<ActiveProxy<'static>> {
    ActiveProxy::builder(c)
        .path(p.clone())?
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

/// Reads the adapters and the access points. With `scan`, asks for a fresh
/// scan first and gives it a few seconds.
pub async fn state(scan: bool) -> Result<WifiState, String> {
    let c = conn().await?;
    let (wifi, wired) = devices(&c).await?;
    let Some(dev) = wifi else {
        return Ok(WifiState {
            wired,
            ..Default::default()
        });
    };
    let w = wireless(&c, &dev).await.map_err(err)?;
    if scan {
        // NM refuses a scan right after another one; the list it has is fine.
        if w.request_scan(HashMap::new()).await.is_ok() {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    }
    let current = w.active_access_point().await.map_err(err)?;
    let mut aps = Vec::new();
    for p in w.get_all_access_points().await.map_err(err)? {
        let Ok(ap) = AccessPointProxy::builder(&c)
            .path(p.clone())
            .map_err(err)?
            .cache_properties(CacheProperties::No)
            .build()
            .await
        else {
            continue;
        };
        // An access point can vanish between the list and the read.
        let (Ok(ssid), Ok(strength), Ok(flags), Ok(wpa), Ok(rsn)) = (
            ap.ssid().await,
            ap.strength().await,
            ap.flags().await,
            ap.wpa_flags().await,
            ap.rsn_flags().await,
        ) else {
            continue;
        };
        aps.push(Network {
            ssid: String::from_utf8_lossy(&ssid).into_owned(),
            ssid_hex: ssid_hex(&ssid),
            strength,
            security: Security::from_flags(flags, wpa, rsn),
            active: p == current,
        });
    }
    let mut st = WifiState {
        available: true,
        wired,
        networks: merge(aps),
        ..Default::default()
    };
    let d = device(&c, &dev).await.map_err(err)?;
    if d.state().await.map_err(err)? == DEVICE_ACTIVATED {
        let ac = d.active_connection().await.map_err(err)?;
        if ac.as_str() != "/" {
            let a = active(&c, &ac).await.map_err(err)?;
            st.active_uuid = a.uuid().await.map_err(err)?;
            st.active_ssid = st
                .networks
                .iter()
                .find(|n| n.active)
                .map(|n| n.ssid.clone())
                .unwrap_or(a.id().await.map_err(err)?);
        }
    }
    Ok(st)
}

/// The settings of a new system-wide connection. The password is stored in
/// the keyfile (`psk-flags` 0), which the helper copies.
fn settings<'a>(
    ssid: &'a [u8],
    id: &'a str,
    uuid: &'a str,
    password: &'a str,
    security: Security,
    hidden: bool,
) -> HashMap<&'static str, HashMap<&'static str, Value<'a>>> {
    let mut s = HashMap::new();
    s.insert(
        "connection",
        HashMap::from([
            ("id", Value::from(id)),
            ("uuid", Value::from(uuid)),
            ("type", Value::from("802-11-wireless")),
            ("autoconnect", Value::from(true)),
        ]),
    );
    s.insert(
        "802-11-wireless",
        HashMap::from([
            ("ssid", Value::from(ssid.to_vec())),
            ("mode", Value::from("infrastructure")),
            ("hidden", Value::from(hidden)),
        ]),
    );
    let sec = match security {
        Security::Psk => Some(HashMap::from([
            ("key-mgmt", Value::from("wpa-psk")),
            ("psk", Value::from(password)),
            ("psk-flags", Value::from(0u32)),
        ])),
        Security::Sae => Some(HashMap::from([
            ("key-mgmt", Value::from("sae")),
            ("psk", Value::from(password)),
            ("psk-flags", Value::from(0u32)),
        ])),
        Security::Wep => Some(HashMap::from([
            ("key-mgmt", Value::from("none")),
            ("wep-key0", Value::from(password)),
            // 1: a hex or ASCII key, 2: a passphrase
            (
                "wep-key-type",
                Value::from(if wep_key(password) { 1u32 } else { 2u32 }),
            ),
            ("wep-key-flags", Value::from(0u32)),
        ])),
        Security::Owe => Some(HashMap::from([("key-mgmt", Value::from("owe"))])),
        Security::Open | Security::Enterprise => None,
    };
    if let Some(sec) = sec {
        s.insert("802-11-wireless-security", sec);
    }
    s.insert("ipv4", HashMap::from([("method", Value::from("auto"))]));
    s.insert("ipv6", HashMap::from([("method", Value::from("auto"))]));
    s
}

/// What NetworkManager would refuse, said before asking it.
fn check_password(ssid: &str, security: Security, password: &str) -> Result<(), String> {
    let ok = match security {
        // 8 to 63 ASCII characters, or 64 hex digits
        Security::Psk => {
            (8..=63).contains(&password.len()) && password.is_ascii()
                || password.len() == 64 && password.bytes().all(|b| b.is_ascii_hexdigit())
        }
        Security::Sae | Security::Wep => !password.is_empty(),
        Security::Open | Security::Owe | Security::Enterprise => true,
    };
    if ok {
        Ok(())
    } else if security == Security::Psk {
        Err(format!(
            "The password for {ssid} is 8 to 63 characters long. Check it and try again."
        ))
    } else {
        Err(format!("{ssid} needs a password."))
    }
}

/// 5 or 13 ASCII characters, or 10 or 26 hex digits.
fn wep_key(k: &str) -> bool {
    matches!(k.len(), 5 | 13)
        || matches!(k.len(), 10 | 26) && k.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A network name is 1 to 32 bytes (not characters).
pub fn check_ssid_len(name: &str) -> Result<(), String> {
    match name.len() {
        0 => Err("Type the network's name.".into()),
        1..=32 => Ok(()),
        n => Err(format!(
            "A network name is at most 32 bytes long. This one is {n} bytes (accented letters and emoji count as more than one)."
        )),
    }
}

/// The security a hidden network is set up with, from the page's choice:
/// "none", "wpa" (WPA and WPA2 Personal) or "sae" (WPA3 Personal). Anything
/// else is guessed from the password, as before the choice existed.
pub fn hidden_security(choice: &str, password: &str) -> Security {
    match choice {
        "none" => Security::Open,
        "wpa" => Security::Psk,
        "sae" => Security::Sae,
        _ if password.is_empty() => Security::Open,
        _ => Security::Psk,
    }
}

/// Connects and waits until NetworkManager says it worked. Returns the
/// new connection's UUID. A failed connection is deleted again. `ssid` is
/// the network's bytes; `hidden` is the security chosen for a network that
/// doesn't show up in the scan.
pub async fn connect(
    ssid: &[u8],
    password: &str,
    hidden: Option<Security>,
) -> Result<String, String> {
    let name = String::from_utf8_lossy(ssid).into_owned();
    let c = conn().await?;
    let (Some(dev), _) = devices(&c).await? else {
        return Err("There is no Wi-Fi adapter.".into());
    };
    let (security, ap) = if let Some(s) = hidden {
        (s, None)
    } else {
        let w = wireless(&c, &dev).await.map_err(err)?;
        let mut best: Option<(u8, Security, OwnedObjectPath)> = None;
        for p in w.get_all_access_points().await.map_err(err)? {
            let Ok(ap) = AccessPointProxy::builder(&c)
                .path(p.clone())
                .map_err(err)?
                .build()
                .await
            else {
                continue;
            };
            let (Ok(s), Ok(st), Ok(f), Ok(wpa), Ok(rsn)) = (
                ap.ssid().await,
                ap.strength().await,
                ap.flags().await,
                ap.wpa_flags().await,
                ap.rsn_flags().await,
            ) else {
                continue;
            };
            if s == ssid && best.as_ref().is_none_or(|b| st > b.0) {
                best = Some((st, Security::from_flags(f, wpa, rsn), p));
            }
        }
        let Some((_, s, p)) = best else {
            return Err(format!("{name} is out of range."));
        };
        (s, Some(p))
    };
    if security == Security::Enterprise {
        return Err(format!(
            "{name} uses WPA Enterprise, which the installer can't set up. Choose another network or set it up after installing."
        ));
    }
    check_password(&name, security, password)?;
    let uuid = new_uuid()?;
    let nm = NetworkManagerProxy::new(&c).await.map_err(err)?;
    let root = ObjectPath::try_from("/").expect("root path");
    let specific = ap.as_ref().map_or(root.clone(), |p| p.as_ref());
    // Listen to the adapter before starting, so the reason it gives for a
    // failure is not missed.
    let reason = watch_reason(&c, &dev).await;
    // each attempt starts with no reason
    *reason.reason.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let (settings_path, active_path) = nm
        .add_and_activate_connection(
            settings(ssid, &name, &uuid, password, security, hidden.is_some()),
            &dev.as_ref(),
            &specific,
        )
        .await
        .map_err(err)?;
    // From here on, a failure deletes the connection again: it holds the
    // password and would keep autoconnecting.
    let r = wait_activated(&c, &active_path, &uuid, &name, security, reason).await;
    if r.is_err()
        && let Ok(b) = SettingsConnectionProxy::builder(&c).path(settings_path.clone())
        && let Ok(s) = b.build().await
    {
        let _ = s.delete().await;
    }
    r
}

/// The reason (NMDeviceStateReason) the adapter last gave for failing or
/// disconnecting, kept up by a task that ends with the connect.
type Reason = std::sync::Arc<std::sync::Mutex<Option<u32>>>;

struct Watch {
    reason: Reason,
    /// The adapter got past the password and is asking for an address.
    ip_config: std::sync::Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn watch_reason(c: &zbus::Connection, dev: &OwnedObjectPath) -> Watch {
    let reason: Reason = Default::default();
    let kept = reason.clone();
    let ip_config = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = ip_config.clone();
    let stream = match device(c, dev).await {
        Ok(d) => d.receive_device_state_changed().await.ok(),
        Err(_) => None,
    };
    let task = tokio::spawn(async move {
        let Some(mut stream) = stream else { return };
        while let Some(sig) = futures_util::StreamExt::next(&mut stream).await {
            let Ok(a) = sig.args() else { continue };
            if a.new_state == DEVICE_IP_CONFIG {
                seen.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if matches!(a.new_state, DEVICE_FAILED | DEVICE_DISCONNECTED)
                && !matches!(
                    a.reason,
                    REASON_NONE | REASON_UNKNOWN | REASON_USER_REQUESTED | REASON_NEW_ACTIVATION
                )
            {
                *kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(a.reason);
            }
        }
    });
    Watch {
        reason,
        ip_config,
        task,
    }
}

// NMDeviceStateReason
const REASON_NONE: u32 = 0;
const REASON_UNKNOWN: u32 = 1;
const REASON_CONFIG_FAILED: u32 = 4;
const REASON_IP_CONFIG_UNAVAILABLE: u32 = 5;
const REASON_IP_CONFIG_EXPIRED: u32 = 6;
const REASON_NO_SECRETS: u32 = 7;
const REASON_SUPPLICANT_DISCONNECT: u32 = 8;
const REASON_SUPPLICANT_CONFIG_FAILED: u32 = 9;
const REASON_SUPPLICANT_FAILED: u32 = 10;
const REASON_SUPPLICANT_TIMEOUT: u32 = 11;
const REASON_DHCP_START_FAILED: u32 = 15;
const REASON_DHCP_ERROR: u32 = 16;
const REASON_DHCP_FAILED: u32 = 17;
const REASON_USER_REQUESTED: u32 = 39;
const REASON_SSID_NOT_FOUND: u32 = 53;
const REASON_NEW_ACTIVATION: u32 = 60;

/// What to tell the user when NetworkManager gave up. Only a secrets
/// failure blames the password; `reason` is `None` when it gave none
/// (we stopped waiting).
fn failure_message(ssid: &str, security: Security, reason: Option<u32>) -> String {
    match reason {
        Some(REASON_NO_SECRETS) if security.needs_password() => {
            format!("Couldn't connect to {ssid}. Check the password and try again.")
        }
        // also what a deauthentication or a weak signal looks like
        Some(REASON_SUPPLICANT_DISCONNECT) if security.needs_password() => format!(
            "Couldn't connect to {ssid}. Check the password, and that the network is in range."
        ),
        // a WPA3 access point that turns the password down shows up as
        // this too: the supplicant only sees it refuse
        Some(REASON_SSID_NOT_FOUND) if security == Security::Sae => format!(
            "Couldn't connect to {ssid}. Check the password, and that the network is in range."
        ),
        Some(REASON_SSID_NOT_FOUND) => {
            format!("Couldn't reach {ssid}. It may be out of range: move closer and try again.")
        }
        Some(REASON_SUPPLICANT_TIMEOUT) | None => {
            format!("Connecting to {ssid} timed out. Try again.")
        }
        Some(
            REASON_DHCP_START_FAILED
            | REASON_DHCP_ERROR
            | REASON_DHCP_FAILED
            | REASON_IP_CONFIG_UNAVAILABLE
            | REASON_IP_CONFIG_EXPIRED,
        ) => format!(
            "Connected to {ssid}, but it didn't give the computer an address (DHCP failed). Try again, or try another network."
        ),
        Some(REASON_SUPPLICANT_CONFIG_FAILED | REASON_SUPPLICANT_FAILED | REASON_CONFIG_FAILED) => {
            format!("Couldn't connect to {ssid}: the Wi-Fi adapter couldn't use these settings.")
        }
        Some(_) => format!("Couldn't connect to {ssid}. Try again, or try another network."),
    }
}

async fn wait_activated(
    c: &zbus::Connection,
    active_path: &OwnedObjectPath,
    uuid: &str,
    ssid: &str,
    security: Security,
    watch: Watch,
) -> Result<String, String> {
    // `timed_out`: we stopped waiting; else NetworkManager gave up, and
    // an unnamed reason is "unknown", not a timeout.
    let failed = |timed_out: bool| {
        let reason = *watch.reason.lock().unwrap_or_else(|e| e.into_inner());
        failure_message(
            ssid,
            security,
            reason.or(if timed_out {
                None
            } else {
                Some(REASON_UNKNOWN)
            }),
        )
    };
    let a = active(c, active_path).await.map_err(|_| failed(false))?;
    // 45 s, or 90 s once the adapter is asking for an address: longer
    // than NetworkManager's own 45 s for DHCP, so its reason comes first
    for i in 0..300 {
        if i >= 150 && !watch.ip_config.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        match a.state().await {
            Ok(ACTIVE_ACTIVATED) => return Ok(uuid.to_string()),
            Ok(ACTIVE_DEACTIVATED) | Err(_) => {
                // a moment for the adapter's last word to arrive
                tokio::time::sleep(Duration::from_millis(300)).await;
                return Err(failed(false));
            }
            Ok(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    Err(failed(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_from_flags() {
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(1, 0, 0), Security::Wep);
        assert_eq!(Security::from_flags(1, 0x188, 0x188), Security::Psk);
        assert_eq!(
            Security::from_flags(1, 0, 0x588),
            Security::Psk,
            "WPA2/WPA3 transition"
        );
        assert_eq!(Security::from_flags(1, 0, 0x488), Security::Sae);
        assert_eq!(Security::from_flags(1, 0, 0x288), Security::Enterprise);
        assert_eq!(Security::from_flags(0, 0, 0x888), Security::Owe);
        assert!(Security::Sae.needs_password() && !Security::Owe.needs_password());
    }

    fn n(ssid: &str, strength: u8, active: bool) -> Network {
        Network {
            ssid: ssid.into(),
            ssid_hex: ssid_hex(ssid.as_bytes()),
            strength,
            security: Security::Psk,
            active,
        }
    }

    #[test]
    fn merges_by_name() {
        let m = merge(vec![
            n("Cafe", 40, false),
            n("Home", 30, false),
            n("", 90, false),
            n("Cafe", 70, false),
            n("Home", 20, true),
            n("Attic", 70, false),
        ]);
        let names: Vec<(&str, u8)> = m.iter().map(|x| (x.ssid.as_str(), x.strength)).collect();
        assert_eq!(names, [("Home", 30), ("Attic", 70), ("Cafe", 70)]);
        assert!(m[0].active);
    }

    #[test]
    fn uuids() {
        let u = new_uuid().unwrap();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(u, new_uuid().unwrap());
    }

    #[test]
    fn passwords() {
        assert!(check_password("n", Security::Psk, "12345678").is_ok());
        assert!(check_password("n", Security::Psk, "1234567").is_err());
        assert!(check_password("n", Security::Psk, &"a".repeat(64)).is_ok());
        assert!(check_password("n", Security::Psk, &"g".repeat(64)).is_err());
        assert!(check_password("n", Security::Sae, "").is_err());
        assert!(check_password("n", Security::Open, "").is_ok());
    }

    #[test]
    fn network_names_are_one_to_thirty_two_bytes() {
        assert!(check_ssid_len("Home").is_ok());
        assert!(check_ssid_len(&"a".repeat(32)).is_ok());
        assert!(check_ssid_len("").is_err());
        assert!(
            check_ssid_len(&"a".repeat(33))
                .unwrap_err()
                .contains("33 bytes")
        );
        // 11 characters, 33 bytes
        assert!(check_ssid_len(&"\u{20ac}".repeat(11)).is_err());
    }

    #[test]
    fn wep_keys() {
        assert!(wep_key("abcde") && wep_key("0123456789") && !wep_key("a passphrase"));
    }

    #[test]
    fn psk_settings_keep_the_password_in_the_keyfile() {
        let s = settings(b"Home", "Home", "u", "secret", Security::Psk, false);
        let sec = &s["802-11-wireless-security"];
        assert_eq!(sec["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(sec["psk-flags"], Value::from(0u32));
        assert!(!s.contains_key("permissions"));
        assert!(
            !settings(b"Open", "Open", "u", "", Security::Open, true)
                .contains_key("802-11-wireless-security")
        );
    }

    #[test]
    fn ssids_keep_their_bytes() {
        // Latin-1 "Caf\xe9": not UTF-8
        let raw = b"Caf\xe9";
        assert_eq!(ssid_hex(raw), "436166e9");
        // the page keys rows and connects by this name
        let json = serde_json::to_value(Network {
            ssid: String::from_utf8_lossy(raw).into(),
            ssid_hex: ssid_hex(raw),
            strength: 50,
            security: Security::Psk,
            active: false,
        })
        .unwrap();
        assert_eq!(json["ssidHex"], "436166e9");
        assert_eq!(ssid_from_hex("436166e9").unwrap(), raw);
        assert_eq!(ssid_from_hex(""), None);
        assert_eq!(ssid_from_hex("abc"), None);
        assert_eq!(ssid_from_hex("zz"), None);
        assert_eq!(ssid_from_hex("é1"), None);
        let net = Network {
            ssid: String::from_utf8_lossy(raw).into_owned(),
            ssid_hex: ssid_hex(raw),
            strength: 50,
            security: Security::Psk,
            active: false,
        };
        assert_eq!(net.ssid, "Caf\u{fffd}");
        // the connection is made with the original bytes, and named lossily
        let s = settings(raw, &net.ssid, "u", "secret", Security::Psk, false);
        assert_eq!(s["802-11-wireless"]["ssid"], Value::from(raw.to_vec()));
        assert_eq!(s["connection"]["id"], Value::from("Caf\u{fffd}"));
    }

    #[test]
    fn networks_that_read_alike_stay_apart() {
        let a = Network {
            ssid_hex: ssid_hex(b"Caf\xe9"),
            ssid: "Caf\u{fffd}".into(),
            ..n("", 40, false)
        };
        let b = Network {
            ssid_hex: ssid_hex(b"Caf\xe8"),
            ..a.clone()
        };
        assert_eq!(merge(vec![a.clone(), b, a]).len(), 2);
        // a name of NULs is a hidden network
        let hidden = Network {
            ssid_hex: ssid_hex(&[0, 0, 0]),
            ssid: "\0\0\0".into(),
            ..n("x", 90, false)
        };
        assert!(merge(vec![hidden]).is_empty());
    }

    #[test]
    fn a_hidden_network_uses_the_chosen_security() {
        assert_eq!(hidden_security("none", "x"), Security::Open);
        assert_eq!(hidden_security("wpa", ""), Security::Psk);
        assert_eq!(hidden_security("sae", "x"), Security::Sae);
        assert_eq!(hidden_security("", ""), Security::Open);
        assert_eq!(hidden_security("", "secret12"), Security::Psk);
        let s = settings(b"H", "H", "u", "secret12", Security::Sae, true);
        assert_eq!(
            s["802-11-wireless-security"]["key-mgmt"],
            Value::from("sae")
        );
        assert_eq!(s["802-11-wireless"]["hidden"], Value::from(true));
        let s = settings(b"H", "H", "u", "secret12", Security::Psk, true);
        assert_eq!(
            s["802-11-wireless-security"]["key-mgmt"],
            Value::from("wpa-psk")
        );
        assert!(
            !settings(b"H", "H", "u", "", Security::Open, true)
                .contains_key("802-11-wireless-security")
        );
    }

    #[test]
    fn only_secret_failures_blame_the_password() {
        let psk = Security::Psk;
        let pw = |r| failure_message("Home", psk, r).contains("password");
        assert!(pw(Some(REASON_NO_SECRETS)));
        assert!(pw(Some(REASON_SUPPLICANT_DISCONNECT)));
        assert!(
            failure_message("Home", psk, Some(REASON_SUPPLICANT_DISCONNECT)).contains("in range")
        );
        assert!(
            !failure_message("Home", psk, Some(REASON_NO_SECRETS)).contains("in range"),
            "only no-secrets is sure it is the password"
        );
        for r in [
            None,
            Some(REASON_SUPPLICANT_TIMEOUT),
            Some(REASON_SSID_NOT_FOUND),
            Some(REASON_DHCP_FAILED),
            Some(REASON_DHCP_ERROR),
            Some(REASON_IP_CONFIG_UNAVAILABLE),
            Some(REASON_SUPPLICANT_FAILED),
            Some(REASON_CONFIG_FAILED),
            Some(999),
        ] {
            assert!(!pw(r), "{r:?}: {}", failure_message("Home", psk, r));
        }
        // an open network has no password to blame
        assert!(
            !failure_message("Cafe", Security::Open, Some(REASON_NO_SECRETS)).contains("password")
        );
        assert!(failure_message("Home", psk, None).contains("timed out"));
        assert!(failure_message("Home", psk, Some(REASON_DHCP_FAILED)).contains("DHCP"));
        assert!(failure_message("Home", psk, Some(REASON_SSID_NOT_FOUND)).contains("out of range"));
        assert!(
            failure_message("Home", Security::Sae, Some(REASON_SSID_NOT_FOUND))
                .contains("password")
        );
    }
}
