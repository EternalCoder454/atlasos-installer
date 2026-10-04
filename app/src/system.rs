//! What the UI reads from the live system itself (unprivileged), and the
//! keyboard layout it applies to the live session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use installer_core::keyboard::{self, Layout};
use installer_core::locales;
use installer_core::settings::Keymap;

/// The UTF-8 locales the image has.
pub fn languages() -> Vec<String> {
    let out = Command::new("/usr/bin/localectl")
        .arg("list-locales")
        .env("SYSTEMD_PAGER", "")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    locales::offered(out.as_deref())
}

pub fn layouts() -> Vec<Layout> {
    let mut l = std::fs::read_to_string(keyboard::BASE_LST)
        .map(|t| keyboard::parse_base_lst(&t))
        .unwrap_or_default();
    if l.is_empty() {
        l.push(Layout {
            id: "us".into(),
            name: "English (US)".into(),
            variants: Vec::new(),
        });
    }
    l
}

/// Secure Boot is on (the SecureBoot EFI variable, readable by anyone).
pub fn secure_boot() -> bool {
    const VAR: &str = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
    // 4 bytes of attributes, then the value
    std::fs::read(VAR).is_ok_and(|b| b.get(4) == Some(&1))
}

/// The live system is the NVIDIA ISO: it carries the module signing key,
/// which the helper queues for enrollment when Secure Boot is on.
pub fn nvidia() -> bool {
    Path::new("/usr/share/atlasos/nvidia/atlasos-module-signing.der").exists()
}

/// Running from the installer media, never an installed system or a
/// developer's desktop. Fails closed: an unreadable `/proc/cmdline` counts
/// as the live session.
pub fn live_session() -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").ok();
    let users: Vec<String> = ["USER", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .collect();
    is_live(
        cmdline.as_deref(),
        &users,
        Path::new("/run/atlas-installer-session").exists(),
        Path::new("/run/initramfs/live").exists(),
    )
}

/// The decision behind [`live_session`], from its inputs: the kernel command
/// line (`None` if unreadable), the user names in the environment, and
/// whether the session's home and the live mount exist.
pub fn is_live(
    cmdline: Option<&str>,
    users: &[String],
    session_dir: bool,
    live_mount: bool,
) -> bool {
    let Some(cmdline) = cmdline else {
        return true;
    };
    cmdline.split_whitespace().any(|w| w == "rd.live.image")
        || users.iter().any(|u| u == "atlas-installer")
        || session_dir
        || live_mount
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// `kxkbrc` for one layout, as Plasma's keyboard settings write it.
pub fn kxkbrc(k: &Keymap) -> String {
    format!(
        "[Layout]\nLayoutList={}\nVariantList={}\nUse=true\n",
        k.layout,
        k.variant.as_deref().unwrap_or("")
    )
}

/// Switches the live session's keyboard: writes `kxkbrc` and tells KWin it
/// changed. Only in the live session; elsewhere it does nothing. Calls run
/// one at a time and only the latest is applied, so quick clicks through the
/// list end on the layout last chosen.
pub async fn apply_keymap(keymap: &str) -> Result<(), String> {
    static LATEST: AtomicU64 = AtomicU64::new(0);
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    if !live_session() {
        return Ok(());
    }
    let k = Keymap::parse(keymap)?;
    let me = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    let _guard = LOCK.lock().await;
    if LATEST.load(Ordering::SeqCst) != me {
        return Ok(());
    }
    let dir = config_dir().ok_or("no config directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(".kxkbrc.atlas-installer");
    std::fs::write(&tmp, kxkbrc(&k)).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join("kxkbrc")).map_err(|e| e.to_string())?;
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    // KConfigWatcher's change notice, as `kwriteconfig6 --notify` sends it:
    // KWin 6.7 reloads the layout on it (the old org.kde.keyboard
    // reloadConfig signal no longer does anything).
    let changed: HashMap<&str, Vec<&[u8]>> =
        HashMap::from([("Layout", vec![&b"LayoutList"[..], b"VariantList", b"Use"])]);
    conn.emit_signal(
        None::<()>,
        "/kxkbrc",
        "org.kde.kconfig.notify",
        "ConfigChanged",
        &(changed,),
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_decision_fails_closed() {
        let none: Vec<String> = Vec::new();
        let me = vec!["zach".to_string()];
        assert!(is_live(None, &me, false, false));
        assert!(is_live(Some("quiet rd.live.image"), &me, false, false));
        assert!(is_live(
            Some("quiet"),
            &["atlas-installer".into()],
            false,
            false
        ));
        assert!(is_live(Some("quiet"), &none, true, false));
        assert!(is_live(Some("quiet"), &none, false, true));
        assert!(!is_live(Some("quiet rd.live.imagex"), &me, false, false));
        assert!(!is_live(Some("quiet ro"), &me, false, false));
    }

    #[test]
    fn kxkbrc_text() {
        assert_eq!(
            kxkbrc(&Keymap::parse("de(nodeadkeys)").unwrap()),
            "[Layout]\nLayoutList=de\nVariantList=nodeadkeys\nUse=true\n"
        );
        assert!(kxkbrc(&Keymap::parse("us").unwrap()).contains("VariantList=\n"));
    }
}
