pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 3 (optional): Wi-Fi. The install needs no internet; the connection
// is copied to the installed system, so first-run setup is online.
InstallerPage {
    id: page

    required property var app

    readonly property var wifi: page.app.wifi
    readonly property bool connected: page.app.wifiUuid.length > 0
    readonly property string connecting: page.app.backend.wifiConnecting
    // The ssidHex of the network whose password field is open; "\n" is
    // "Other network". (The hex, not the name: names can look alike.)
    property string open: ""
    // The rows shown. Not updated while a password field is open: a new
    // model would rebuild the rows and lose what was typed.
    property var networks: []

    function syncNetworks() {
        if (page.open === "") {
            page.networks = (page.wifi.networks || []).concat([{ ssid: "\n", ssidHex: "", strength: 0, security: "psk", active: false }]);
        }
    }
    onWifiChanged: page.syncNetworks()
    onOpenChanged: page.syncNetworks()

    title: qsTr("Connect to Wi-Fi")
    subtitle: page.connected ? qsTr("AtlasOS will connect to %1 after it starts.").arg(page.wifi.activeSsid) : qsTr("Connect now so AtlasOS can finish setting up after it starts. You can also do this later: the install itself needs no internet.")
    primaryText: page.connected ? qsTr("Continue") : qsTr("Set Up Later")
    primaryEnabled: page.connecting.length === 0
    onPrimary: page.app.next()
    onBack: page.app.previous()

    function signalIcon(s) {
        if (s >= 75) return "network-wireless-signal-excellent-symbolic";
        if (s >= 50) return "network-wireless-signal-good-symbolic";
        if (s >= 25) return "network-wireless-signal-ok-symbolic";
        return "network-wireless-signal-weak-symbolic";
    }

    function describe(n) {
        if (n.active) return qsTr("Connected");
        if (n.ssidHex === page.connecting) return qsTr("Connecting…");
        switch (n.security) {
        case "open": return qsTr("Open network");
        case "owe": return qsTr("Open network, encrypted");
        case "wep": return qsTr("Secured with WEP (outdated)");
        case "enterprise": return qsTr("WPA Enterprise: set it up after installing");
        default: return qsTr("Secured");
        }
    }

    function pick(n) {
        page.app.backend.clearWifiError();
        if (n.active || n.security === "enterprise") {
            page.open = "";
        } else if (n.security === "open" || n.security === "owe") {
            page.open = "";
            page.app.backend.connectWifi(n.ssid, n.ssidHex, "", false, "");
        } else {
            page.open = page.open === n.ssidHex ? "" : n.ssidHex;
        }
    }

    Component.onCompleted: {
        page.app.backend.clearWifiError();
        page.syncNetworks();
        page.app.backend.refreshWifi(true);
        // Demo screenshots: the form for a hidden network, open
        if (page.app.backend.demo && page.app.demoHidden) {
            page.open = "\n";
        }
    }

    Timer {
        interval: 15000
        repeat: true
        running: page.visible && page.connecting.length === 0 && page.open.length === 0
        onTriggered: page.app.backend.refreshWifi(true)
    }

    // A connection that worked closes the password field.
    Connections {
        target: page.app.backend
        function onWifiConnectingChanged() {
            if (page.app.backend.wifiConnecting.length === 0 && page.app.backend.wifiError.length === 0) {
                page.open = "";
            }
        }
    }

    component PasswordBox: ColumnLayout {
        id: box
        property string ssid
        property string ssidHex
        property bool hidden: false
        property bool needsPassword: true
        // A hidden network doesn't say how it is secured: the user does.
        readonly property bool noPassword: box.hidden && security.currentValue === "none"
        spacing: Kirigami.Units.largeSpacing
        Layout.fillWidth: true

        function go() {
            const name = box.hidden ? nameField.text.trim() : box.ssid;
            if (name.length > 0) {
                page.app.backend.connectWifi(name, box.ssidHex, box.noPassword ? "" : password.text, box.hidden, box.hidden ? security.currentValue : "");
            }
        }

        Component.onCompleted: (box.hidden ? nameField : password).forceActiveFocus()

        RowLayout {
            visible: box.hidden
            spacing: Kirigami.Units.largeSpacing
            Layout.fillWidth: true
            AtlasTextField {
                id: nameField
                Layout.fillWidth: true
                placeholderText: qsTr("Network name")
                Accessible.name: placeholderText
                enabled: page.connecting.length === 0
                onAccepted: password.forceActiveFocus()
            }
            AtlasComboBox {
                id: security
                Layout.preferredWidth: Kirigami.Units.gridUnit * 14
                enabled: page.connecting.length === 0
                textRole: "text"
                valueRole: "value"
                model: [
                    { text: qsTr("WPA / WPA2 Personal"), value: "wpa" },
                    { text: qsTr("WPA3 Personal"), value: "sae" },
                    { text: qsTr("None (open network)"), value: "none" }
                ]
                Accessible.name: qsTr("Security")
            }
        }
        RowLayout {
            spacing: Kirigami.Units.largeSpacing
            Layout.fillWidth: true
            AtlasPasswordField {
                id: password
                Layout.fillWidth: true
                placeholderText: qsTr("Password")
                Accessible.name: placeholderText
                enabled: page.connecting.length === 0 && !box.noPassword
                onAccepted: box.go()
            }
            PrimaryButton {
                text: page.connecting.length > 0 ? qsTr("Connecting…") : qsTr("Connect")
                enabled: page.connecting.length === 0 && (box.hidden ? nameField.text.trim().length > 0 && (box.noPassword || password.text.length > 0) : password.text.length >= (box.needsPassword ? 1 : 0))
                onClicked: box.go()
            }
        }
    }

    ColumnLayout {
        width: page.contentWidth
        height: parent.height - Kirigami.Units.gridUnit
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Kirigami.Units.largeSpacing

        RowLayout {
            Layout.fillWidth: true
            QQC2.Label {
                Layout.fillWidth: true
                text: qsTr("Networks")
                font.bold: true
                opacity: 0.65
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Accessible.role: Accessible.Heading
            }
            // Only while there's nothing to list yet: the rescan every 15 s
            // just updates the rows, without a spinner redrawing the page.
            AtlasSpinner {
                running: page.app.backend.wifiScanning && (page.wifi.networks || []).length === 0
                implicitWidth: Kirigami.Units.iconSizes.small
                implicitHeight: implicitWidth
            }
        }

        ListFrame {
            id: frame
            Layout.fillWidth: true
            Layout.fillHeight: true
            model: page.networks
            delegate: ListRow {
                id: netRow
                required property var modelData
                readonly property bool other: modelData.ssid === "\n"
                title: other ? qsTr("Other Network…") : modelData.ssid
                subtitle: other ? qsTr("A network that doesn't show its name") : page.describe(modelData)
                iconName: other ? "network-wireless-hidden-symbolic" : page.signalIcon(modelData.strength)
                selected: !other && modelData.active
                clickable: other || modelData.security !== "enterprise"
                expanded: page.open === (netRow.other ? "\n" : netRow.modelData.ssidHex)
                onClicked: {
                    if (other) {
                        page.app.backend.clearWifiError();
                        page.open = page.open === "\n" ? "" : "\n";
                    } else {
                        page.pick(modelData);
                    }
                }

                Kirigami.Icon {
                    visible: !netRow.other && ["psk", "sae", "wep", "enterprise"].includes(netRow.modelData.security)
                    source: "object-locked-symbolic"
                    isMask: true
                    color: Kirigami.Theme.textColor
                    opacity: 0.5
                    width: Kirigami.Units.iconSizes.small
                    height: width
                }

                expansion: Loader {
                    Layout.fillWidth: true
                    active: netRow.expanded
                    sourceComponent: PasswordBox {
                        ssid: netRow.other ? "" : netRow.modelData.ssid
                        ssidHex: netRow.other ? "" : netRow.modelData.ssidHex
                        hidden: netRow.other
                    }
                }
            }
        }

        InfoBanner {
            Layout.fillWidth: true
            shown: page.app.backend.wifiError.length > 0
            type: "error"
            text: page.app.backend.wifiError
        }
    }
}
