pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as QQC2
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import Atlas.Ui

QQC2.ApplicationWindow {
    id: root

    // Both come from main.cpp (setInitialProperties).
    required property var backend
    required property bool fullScreen
    // Demo mode: the step to open at ("" for the first); "wifi-hidden"
    // opens the Wi-Fi page with the form for a hidden network.
    required property string demoPage
    readonly property bool demoHidden: root.demoPage === "wifi-hidden" || root.demoHiddenKept
    property bool demoHiddenKept: false

    title: qsTr("Install AtlasOS")
    width: Kirigami.Units.gridUnit * 64
    height: Kirigami.Units.gridUnit * 42
    minimumWidth: Kirigami.Units.gridUnit * 46
    minimumHeight: Kirigami.Units.gridUnit * 32
    visibility: root.fullScreen ? QQC2.ApplicationWindow.FullScreen : QQC2.ApplicationWindow.Windowed
    visible: true
    color: Kirigami.Theme.backgroundColor

    LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
    LayoutMirroring.childrenInherit: true

    // What the user chose. The pages read and set these.
    property string language: "en_US.UTF-8"
    property string keymap: "us"
    // The user picked a layout: a new language no longer changes it.
    property bool keymapChosen: false
    // A view::DiskRow, or null.
    property var disk: null
    // "erase" or "free-space"
    property string mode: ""
    // Disk encryption: "" until the user touches the switch (then the PC's
    // TPM decides), else "on" or "off". `encryptPin`: with a TPM, also ask
    // for a PIN at start-up. The start-up password or PIN lives only here,
    // while the user types it: it is handed to Install and wiped.
    property string encryptChoice: ""
    // Same for the PIN: "" lets the PC decide (a PIN when there is a TPM but
    // Secure Boot is off), else "on" or "off".
    property string pinChoice: ""
    readonly property bool encryptPin: root.pinChoice === "" ? root.backend.pinDefault(root.backend.tpm2, root.backend.secureBootOff) : root.pinChoice === "on"
    // Why Install was refused, shown on the Review page.
    property string installRefusal: ""
    property string password: ""
    property string passwordConfirm: ""
    readonly property bool encrypt: root.encryptChoice === "" ? root.backend.encryptionDefault(root.backend.tpm2) : root.encryptChoice === "on"
    readonly property string encryption: root.backend.encryptionMode(root.backend.tpm2, root.encrypt, root.encryptPin)
    // A password or PIN is typed for these.
    readonly property bool needsSecret: root.encryption === "password" || root.encryption === "tpm-pin"
    // Continue on the Disk page: a password or PIN, if one is needed, is valid.
    readonly property bool encryptionReady: !root.needsSecret || root.backend.secretOk(root.encryption, root.password, root.passwordConfirm)
    // Not kept for a disk that isn't encrypted with it, and a password that
    // was typed must never silently become the PIN (or the other way round).
    onEncryptionChanged: root.wipePassword()

    function wipePassword() {
        root.password = "";
        root.passwordConfirm = "";
    }

    readonly property var wifi: JSON.parse(root.backend.wifiJson)
    // No adapter, or a cable: nothing to set up. Decided once, at the first
    // read, so the steps never change under the user.
    property bool skipWifi: false
    // The connection Install copies: the one made on the Wi-Fi page, else
    // the one already active.
    readonly property string wifiUuid: root.backend.wifiUuid || root.wifi.activeUuid || ""
    readonly property string installState: root.backend.installState

    readonly property var allSteps: [
        { key: "welcome", title: qsTr("Welcome") },
        { key: "keyboard", title: qsTr("Keyboard") },
        { key: "wifi", title: qsTr("Wi-Fi") },
        { key: "disk", title: qsTr("Disk") },
        { key: "review", title: qsTr("Review") },
        { key: "progress", title: qsTr("Install") },
        { key: "restart", title: qsTr("Restart") }
    ]
    readonly property var steps: root.allSteps.filter(s => s.key !== "wifi" || !root.skipWifi)
    property string current: "welcome"
    // The furthest step reached, as an index into allSteps (which never
    // changes), so the step panel can go back to done ones.
    property int reached: 0
    readonly property int currentIndex: root.steps.findIndex(s => s.key === root.current)

    function allIndex(key) {
        return root.allSteps.findIndex(s => s.key === key);
    }
    readonly property bool canGoBack: root.installState === "idle"

    readonly property var pages: ({
            "welcome": welcomePage,
            "keyboard": keyboardPage,
            "wifi": wifiPage,
            "disk": diskPage,
            "review": reviewPage,
            "progress": progressPage,
            "restart": restartPage
        })

    function show(key) {
        if (key === root.current && stack.depth > 0) {
            return;
        }
        if (root.steps.findIndex(s => s.key === key) < 0) {
            return;
        }
        const forward = root.allIndex(key) > root.allIndex(root.current);
        if (root.current === "disk" && !forward) {
            root.wipePassword();
        }
        root.installRefusal = "";
        root.current = key;
        root.reached = Math.max(root.reached, root.allIndex(key));
        stack.replaceCurrentItem(root.pages[key], {}, forward ? QQC2.StackView.PushTransition : QQC2.StackView.PopTransition);
    }

    function next() {
        const i = root.currentIndex;
        if (i + 1 < root.steps.length) {
            root.show(root.steps[i + 1].key);
        }
    }

    function previous() {
        const i = root.currentIndex;
        if (i > 0 && root.canGoBack) {
            root.show(root.steps[i - 1].key);
        }
    }

    // The chosen disk is still in the latest list, unchanged, and allows the mode.
    function diskStillOk() {
        if (root.disk === null || root.backend.disksState !== "ready") {
            return false;
        }
        const d = JSON.parse(root.backend.disksJson).find(r => r.id === root.disk.id);
        return d !== undefined && d.fingerprint === root.disk.fingerprint && d.selectable
            && (root.mode === "erase" && d.eraseOk || root.mode === "free-space" && d.freeOk);
    }

    function startInstall() {
        if (!root.diskStillOk()) {
            root.chooseDiskAgain();
            return;
        }
        if (!root.encryptionReady) {
            root.show("disk");
            return;
        }
        root.installRefusal = "";
        const accepted = root.backend.install(root.disk.id, root.disk.fingerprint, root.mode, root.language, root.keymap, root.wifiUuid, root.encryption, root.needsSecret ? root.password : "");
        if (!accepted) {
            // The password stays, so the user can try again.
            root.show("review");
            root.installRefusal = root.installState !== "idle" ? qsTr("An install is already running, or the PC is restarting.") : qsTr("AtlasOS couldn't start the install with these choices. Go back and check the disk and the password or PIN, then try again.");
            return;
        }
        root.wipePassword();
        root.show("progress");
    }

    // After a failed install, or when the disk changed: back to the Disk step.
    function chooseDiskAgain() {
        root.backend.resetInstall();
        root.disk = null;
        root.mode = "";
        root.reached = root.allIndex("disk");
        root.backend.refreshDisks();
        root.show("disk");
    }

    // Demo screenshots: open at demoPage once the disks and networks are
    // read. Past the Disk page, the first disk is chosen; the Disk page
    // itself opens as a user would reach it.
    function openDemoPage() {
        let target = root.demoPage;
        if (target === "wifi-hidden") {
            root.demoHiddenKept = true;
            target = "wifi";
        }
        if (!root.backend.demo || target === "" || root.allSteps.findIndex(s => s.key === target) < 0) {
            return;
        }
        if (root.backend.disksState === "loading" || !root.backend.wifiLoaded) {
            return;
        }
        root.demoPage = "";
        if (root.backend.demoEncrypt) {
            root.encryptChoice = "on";
        }
        root.pinChoice = root.backend.demoPin ? "on" : "";
        const disks = JSON.parse(root.backend.disksJson);
        if (disks.length > 0 && target !== "disk") {
            root.disk = disks[0];
            root.mode = disks[0].freeOk ? "free-space" : "erase";
        }
        if (target === "wifi" && root.skipWifi) {
            root.show("disk");
            return;
        }
        const later = ["review", "progress", "restart"];
        if (later.includes(target) && root.disk === null) {
            root.show("disk");
            return;
        }
        if (target === "progress" || target === "restart") {
            root.show("review");
            root.startInstall();
        } else {
            root.show(target);
        }
    }

    Component.onCompleted: {
        root.backend.start();
        stack.push(welcomePage, {}, QQC2.StackView.Immediate);
    }

    // The install keeps going in the helper if the window closes, but the
    // user would lose sight of it.
    onClosing: close => {
        if (root.installState === "running") {
            close.accepted = false;
        }
    }

    Connections {
        target: root.backend
        enabled: root.demoPage !== ""
        function onDisksStateChanged() {
            root.openDemoPage();
        }
    }

    Connections {
        target: root.backend
        function onWifiLoadedChanged() {
            if (!root.backend.wifiLoaded) {
                return;
            }
            root.skipWifi = !root.wifi.available || root.wifi.wired;
            // Already on the page when the answer came: move on.
            if (root.skipWifi && root.current === "wifi") {
                root.show("disk");
            }
            root.openDemoPage();
        }
        function onInstallStateChanged() {
            // Also when the helper was already installing as we started
            const st = root.backend.installState;
            if (st === "done") {
                root.show("restart");
            } else if (st === "running" || st === "failed") {
                root.show("progress");
            }
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        BrandPanel {
            Layout.fillHeight: true
            Layout.preferredWidth: Math.round(Math.max(Kirigami.Units.gridUnit * 13, Math.min(Kirigami.Units.gridUnit * 19, root.width * 0.3)))
            steps: root.steps
            current: root.current
            // reached counts allSteps; the panel counts the shown steps.
            reached: root.steps.reduce((n, s, i) => root.allIndex(s.key) <= root.reached ? i : n, 0)
            canGoBack: root.canGoBack
            demo: root.backend.demo
            onStepClicked: key => root.show(key)
        }

        QQC2.StackView {
            id: stack
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            readonly property real slide: Kirigami.Units.gridUnit * 3
            replaceEnter: Transition {
                ParallelAnimation {
                    NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                    NumberAnimation { property: "x"; from: stack.slide; to: 0; duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                }
            }
            replaceExit: Transition {
                NumberAnimation { property: "opacity"; from: 1; to: 0; duration: Kirigami.Units.shortDuration }
            }
            pushEnter: replaceEnter
            pushExit: replaceExit
            popEnter: Transition {
                ParallelAnimation {
                    NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                    NumberAnimation { property: "x"; from: -stack.slide; to: 0; duration: Kirigami.Units.longDuration; easing.type: Easing.OutCubic }
                }
            }
            popExit: replaceExit
        }
    }

    Component {
        id: welcomePage
        WelcomePage {
            app: root
        }
    }
    Component {
        id: keyboardPage
        KeyboardPage {
            app: root
        }
    }
    Component {
        id: wifiPage
        WifiPage {
            app: root
        }
    }
    Component {
        id: diskPage
        DiskPage {
            app: root
        }
    }
    Component {
        id: reviewPage
        ReviewPage {
            app: root
        }
    }
    Component {
        id: progressPage
        ProgressPage {
            app: root
        }
    }
    Component {
        id: restartPage
        RestartPage {
            app: root
        }
    }
}
