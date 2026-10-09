//! Telamon Installer, Rust side. `cpp/` only starts Qt and loads the QML;
//! everything QML talks to is the `Backend` QObject in `backend.rs`.

// Install takes everything the helper does, as QML hands it over.
#![allow(clippy::too_many_arguments)]

mod backend;
mod demo;
mod helper;
mod network;
mod system;
mod view;

use std::ffi::c_void;

/// Zero a buffer in a way the compiler can't drop as a dead store.
pub(crate) fn wipe_bytes(b: &mut [u8]) {
    for x in b.iter_mut() {
        // SAFETY: `x` is a valid, aligned, exclusive reference
        unsafe { std::ptr::write_volatile(x, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

/// Overwrite a password's copy before it is freed (the helper does the same
/// with its own). The QML property and Qt's string keep theirs until the
/// page goes; this is the copy the Rust side made, which would otherwise
/// stay in freed memory.
pub(crate) fn wipe(s: String) {
    let mut b = s.into_bytes();
    wipe_bytes(&mut b);
}

/// Called once from `main.cpp`. Returns the `Backend` QObject (no parent;
/// the caller owns it).
#[unsafe(no_mangle)]
pub extern "C" fn telamon_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}

#[cfg(test)]
mod tests {
    /// Every QML file of the pages. SSIDs, disk model names and serials come
    /// from other people's devices: a label that guessed "rich text" from
    /// them could show markup, or fetch an image from a server.
    const QML: [(&str, &str); 15] = [
        ("AppsPage", include_str!("../qml/AppsPage.qml")),
        ("BackButton", include_str!("../qml/BackButton.qml")),
        ("BrandPanel", include_str!("../qml/BrandPanel.qml")),
        ("ChoiceCard", include_str!("../qml/ChoiceCard.qml")),
        ("DiskPage", include_str!("../qml/DiskPage.qml")),
        ("InstallerPage", include_str!("../qml/InstallerPage.qml")),
        ("KeyboardPage", include_str!("../qml/KeyboardPage.qml")),
        ("ListFrame", include_str!("../qml/ListFrame.qml")),
        ("ListRow", include_str!("../qml/ListRow.qml")),
        ("Main", include_str!("../qml/Main.qml")),
        ("ProgressPage", include_str!("../qml/ProgressPage.qml")),
        ("RestartPage", include_str!("../qml/RestartPage.qml")),
        ("ReviewPage", include_str!("../qml/ReviewPage.qml")),
        ("WelcomePage", include_str!("../qml/WelcomePage.qml")),
        ("WifiPage", include_str!("../qml/WifiPage.qml")),
    ];

    #[test]
    fn every_label_shows_plain_text() {
        let mut seen = 0;
        for (file, src) in QML {
            for kind in ["QQC2.Label {", "Text {"] {
                let mut from = 0;
                while let Some(at) = src[from..].find(kind) {
                    let start = from + at;
                    from = start + kind.len();
                    // a word that merely ends in "Text" (a property, a component)
                    if src[..start].ends_with(|c: char| c.is_alphanumeric() || c == '.')
                        && kind == "Text {"
                    {
                        continue;
                    }
                    let mut depth = 1;
                    let mut end = from;
                    for (i, c) in src[from..].char_indices() {
                        depth += i32::from(c == '{') - i32::from(c == '}');
                        if depth == 0 {
                            end = from + i;
                            break;
                        }
                    }
                    let block = &src[start..end];
                    assert!(
                        block.contains("textFormat: Text.PlainText"),
                        "{file}.qml: a label without textFormat: Text.PlainText:\n{block}"
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen > 25, "{seen} labels looked at");
    }

    #[test]
    fn no_page_opens_links_loads_code_or_runs_a_program() {
        for (file, src) in QML {
            for bad in [
                "Qt.openUrlExternally",
                "createQmlObject",
                "Qt.createComponent",
                "eval(",
                "Qt.include",
                "onLinkActivated",
                "RichText",
                "StyledText",
                "AutoText",
                "Process",
                "XMLHttpRequest",
                "WebView",
                "Qt.labs.platform",
            ] {
                assert!(!src.contains(bad), "{file}.qml uses {bad}");
            }
        }
    }

    #[test]
    fn wiping_zeroes_the_whole_buffer() {
        let mut b = b"CANARY-pw-7f3a".to_vec();
        super::wipe_bytes(&mut b);
        assert_eq!(b, [0; 14]);
        super::wipe("CANARY-pin".to_string());
    }
}
