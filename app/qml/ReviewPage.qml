pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui
import net.eterneon.atlas.installer

// Step 5: everything in plain words, and exactly what happens to the disk.
InstallerPage {
    id: page

    required property var app

    readonly property var disk: page.app.disk
    readonly property bool erase: page.app.mode === "erase"
    // Erasing a disk with nothing found on it: no warning. It is still
    // confirmed, since data lsblk can't recognise looks empty too.
    readonly property bool losesData: page.erase && page.disk !== null && !page.disk.empty
    readonly property var layouts: JSON.parse(page.app.backend.layoutsJson)

    title: qsTr("Review")
    subtitle: qsTr("Check everything before AtlasOS is installed.")
    primaryText: page.losesData ? qsTr("Erase and Install") : qsTr("Install")
    primaryEnabled: page.disk !== null && page.app.mode.length > 0
    onPrimary: page.erase ? confirm.open() : page.app.startInstall()
    onBack: page.app.previous()

    function keyboardName() {
        const id = page.app.keymap.split("(")[0];
        const v = page.app.keymap.includes("(") ? page.app.keymap.split("(")[1].replace(")", "") : "";
        const l = page.layouts.find(x => x.id === id);
        if (!l) {
            return page.app.keymap;
        }
        const variant = l.variants.find(x => x.id === v);
        return variant ? variant.name : l.name;
    }

    readonly property string wifiText: {
        const w = page.app.wifi;
        if (w.wired) return qsTr("Connected by cable");
        if (page.app.wifiUuid.length > 0) return w.activeSsid || qsTr("Connected");
        return qsTr("Not set up");
    }

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            width: page.contentWidth
            x: Math.round((scroll.width - width) / 2)
            spacing: Kirigami.Units.gridUnit

            Section {
                SectionRow {
                    iconName: "preferences-desktop-locale"
                    title: qsTr("Language")
                    value: LocaleInfo.describe(page.app.language).native
                }
                SectionRow {
                    iconName: "input-keyboard"
                    title: qsTr("Keyboard")
                    value: page.keyboardName()
                }
                SectionRow {
                    // No adapter: nothing to say.
                    visible: !page.app.skipWifi || page.app.wifi.wired === true
                    iconName: "network-wireless"
                    title: qsTr("Wi-Fi")
                    value: page.wifiText
                }
                SectionRow {
                    iconName: page.disk ? page.disk.icon : "drive-harddisk"
                    title: qsTr("Disk")
                    value: page.disk ? page.disk.title + " (" + page.disk.subtitle.split(" · ")[0] + ")" : ""
                }
            }

            Note {
                Layout.fillWidth: true
                visible: page.disk !== null
                kind: page.losesData ? "warning" : "info"
                text: !page.disk ? "" : !page.erase ? page.disk.reviewFree : page.losesData ? page.disk.reviewErase + " " + qsTr("This can't be undone.") : page.disk.reviewErase
            }

            Note {
                Layout.fillWidth: true
                visible: page.app.backend.nvidia && page.app.backend.secureBoot
                kind: "info"
                text: qsTr("Secure Boot is on, so the NVIDIA driver needs the AtlasOS key. After the restart, a blue screen asks you to enroll it, once. The last page shows the password and the steps.")
            }

            Note {
                Layout.fillWidth: true
                visible: page.disk !== null && page.disk.note.length > 0
                kind: "warning"
                text: page.disk ? page.disk.note : ""
            }

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                text: qsTr("Installing takes a few minutes and can't be stopped once it has started. You'll create your account after the restart.")
                wrapMode: Text.Wrap
                opacity: 0.65
                font: Kirigami.Theme.smallFont
            }
        }
    }

    ConfirmDialog {
        id: confirm
        title: !page.disk ? "" : page.losesData ? qsTr("Erase %1?").arg(page.disk.title) : qsTr("Use All of %1?").arg(page.disk.title)
        text: !page.disk ? "" : page.losesData ? page.disk.eraseText + " " + qsTr("This can't be undone.") : qsTr("Nothing was found on this disk. If it does hold anything, it is erased.")
        acceptText: page.losesData ? qsTr("Erase and Install") : qsTr("Install")
        focusReject: true
        onAccepted: page.app.startInstall()
    }
}
