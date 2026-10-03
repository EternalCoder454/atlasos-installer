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
    // Demo mode: the step to open at ("" for the first).
    required property string demoPage

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
    // changes), so the sidebar can go back to done ones.
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
        root.show("progress");
        root.backend.install(root.disk.id, root.disk.fingerprint, root.mode, root.language, root.keymap, root.wifiUuid);
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

    // Demo screenshots: pick the first disk and open at demoPage once the
    // disks and networks are read.
    function openDemoPage() {
        const target = root.demoPage;
        if (!root.backend.demo || target === "" || root.allSteps.findIndex(s => s.key === target) < 0) {
            return;
        }
        if (root.backend.disksState === "loading" || !root.backend.wifiLoaded) {
            return;
        }
        root.demoPage = "";
        const disks = JSON.parse(root.backend.disksJson);
        if (disks.length > 0) {
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
            if (root.backend.installState === "done") {
                root.show("restart");
            }
        }
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        Rectangle {
            id: sidebar
            Layout.fillHeight: true
            Layout.preferredWidth: Kirigami.Units.gridUnit * 13
            color: Qt.tint(Kirigami.Theme.backgroundColor, Qt.alpha(Kirigami.Theme.highlightColor, 0.07))

            Rectangle {
                anchors.right: parent.right
                height: parent.height
                width: 1
                color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
            }

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Kirigami.Units.largeSpacing
                anchors.rightMargin: Kirigami.Units.largeSpacing + 1
                anchors.topMargin: Kirigami.Units.gridUnit * 1.5
                spacing: Kirigami.Units.smallSpacing

                RowLayout {
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.bottomMargin: Kirigami.Units.gridUnit
                    spacing: Kirigami.Units.largeSpacing
                    Image {
                        source: "qrc:/qt/qml/net/eterneon/atlas/installer/data/atlasos-logo.svg"
                        sourceSize.width: Kirigami.Units.iconSizes.medium
                        sourceSize.height: Kirigami.Units.iconSizes.medium
                        Accessible.ignored: true
                    }
                    QQC2.Label {
                        text: "AtlasOS"
                        font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.3
                        font.weight: Font.DemiBold
                        textFormat: Text.PlainText
                    }
                }

                Repeater {
                    model: root.steps
                    StepItem {
                        id: stepItem
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        number: stepItem.index + 1
                        text: stepItem.modelData.title
                        current: root.current === stepItem.modelData.key
                        readonly property int allIndex: root.allIndex(stepItem.modelData.key)
                        done: stepItem.allIndex <= root.reached && !stepItem.current
                        clickable: stepItem.done && root.canGoBack && stepItem.allIndex < root.allIndex(root.current)
                        // Space still clicks a focused button, clickable or not.
                        onClicked: if (stepItem.clickable) root.show(stepItem.modelData.key)
                    }
                }

                Item {
                    Layout.fillHeight: true
                }

                QQC2.Label {
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    visible: root.backend.demo
                    text: qsTr("Demo: nothing here is real, and nothing is installed.")
                    wrapMode: Text.Wrap
                    font: Kirigami.Theme.smallFont
                    color: Kirigami.Theme.neutralTextColor
                    textFormat: Text.PlainText
                }
            }
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
