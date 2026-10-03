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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Network {
    pub ssid: String,
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
        if ap.ssid.trim().is_empty() {
            continue;
        }
        match out.iter_mut().find(|n| n.ssid == ap.ssid) {
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
    ssid: &'a str,
    uuid: &'a str,
    password: &'a str,
    security: Security,
    hidden: bool,
) -> HashMap<&'static str, HashMap<&'static str, Value<'a>>> {
    let mut s = HashMap::new();
    s.insert(
        "connection",
        HashMap::from([
            ("id", Value::from(ssid)),
            ("uuid", Value::from(uuid)),
            ("type", Value::from("802-11-wireless")),
            ("autoconnect", Value::from(true)),
        ]),
    );
    s.insert(
        "802-11-wireless",
        HashMap::from([
            ("ssid", Value::from(ssid.as_bytes().to_vec())),
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

/// Connects and waits until NetworkManager says it worked. Returns the
/// new connection's UUID. A failed connection is deleted again.
pub async fn connect(ssid: &str, password: &str, hidden: bool) -> Result<String, String> {
    let c = conn().await?;
    let (Some(dev), _) = devices(&c).await? else {
        return Err("There is no Wi-Fi adapter.".into());
    };
    let (security, ap) = if hidden {
        let s = if password.is_empty() {
            Security::Open
        } else {
            Security::Psk
        };
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
            if s == ssid.as_bytes() && best.as_ref().is_none_or(|b| st > b.0) {
                best = Some((st, Security::from_flags(f, wpa, rsn), p));
            }
        }
        let Some((_, s, p)) = best else {
            return Err(format!("{ssid} is out of range."));
        };
        (s, Some(p))
    };
    if security == Security::Enterprise {
        return Err(format!(
            "{ssid} uses WPA Enterprise, which the installer can't set up. Choose another network or set it up after installing."
        ));
    }
    // A hidden network's security is a guess (WPA2): only check what it is sure of.
    if !hidden {
        check_password(ssid, security, password)?;
    }
    let uuid = new_uuid()?;
    let nm = NetworkManagerProxy::new(&c).await.map_err(err)?;
    let root = ObjectPath::try_from("/").expect("root path");
    let specific = ap.as_ref().map_or(root.clone(), |p| p.as_ref());
    let (settings_path, active_path) = nm
        .add_and_activate_connection(
            settings(ssid, &uuid, password, security, hidden),
            &dev.as_ref(),
            &specific,
        )
        .await
        .map_err(err)?;
    // From here on, a failure deletes the connection again: it holds the
    // password and would keep autoconnecting.
    let r = wait_activated(&c, &active_path, &uuid, ssid, security).await;
    if r.is_err()
        && let Ok(b) = SettingsConnectionProxy::builder(&c).path(settings_path.clone())
        && let Ok(s) = b.build().await
    {
        let _ = s.delete().await;
    }
    r
}

async fn wait_activated(
    c: &zbus::Connection,
    active_path: &OwnedObjectPath,
    uuid: &str,
    ssid: &str,
    security: Security,
) -> Result<String, String> {
    let failed = || {
        if security.needs_password() {
            format!("Couldn't connect to {ssid}. Check the password and try again.")
        } else {
            format!("Couldn't connect to {ssid}.")
        }
    };
    let a = active(c, active_path).await.map_err(|_| failed())?;
    for _ in 0..150 {
        match a.state().await {
            Ok(ACTIVE_ACTIVATED) => return Ok(uuid.to_string()),
            Ok(ACTIVE_DEACTIVATED) | Err(_) => return Err(failed()),
            Ok(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    Err(failed())
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
    fn wep_keys() {
        assert!(wep_key("abcde") && wep_key("0123456789") && !wep_key("a passphrase"));
    }

    #[test]
    fn psk_settings_keep_the_password_in_the_keyfile() {
        let s = settings("Home", "u", "secret", Security::Psk, false);
        let sec = &s["802-11-wireless-security"];
        assert_eq!(sec["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(sec["psk-flags"], Value::from(0u32));
        assert!(!s.contains_key("permissions"));
        assert!(
            !settings("Open", "u", "", Security::Open, true)
                .contains_key("802-11-wireless-security")
        );
    }
}
