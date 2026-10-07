pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Telamon.Ui

// Step 8: done. Restart, plus the NVIDIA key steps when the helper queued
// the key, and anything that went wrong without failing the install.
InstallerPage {
    id: page

    required property var app

    readonly property var backend: page.app.backend
    readonly property var result: JSON.parse(page.backend.resultJson)
    readonly property string mok: page.result.mokPassword || ""
    readonly property string recoveryKey: page.result.recoveryKey || ""
    // The key was shown to be written down: Restart waits for the tick.
    property bool keySaved: false

    title: qsTr("Telamon OS is installed")
    // Afterwards, not before: the helper restarts without the stick, but the
    // firmware is told to start Telamon OS next either way, so it can stay in.
    subtitle: page.result.bootMedia === "cd" ? qsTr("Restart, then take out the disc while Telamon OS starts.")
            : page.result.bootMedia === "usb" ? qsTr("Restart, then take out the USB stick while Telamon OS starts.")
            : qsTr("Restart, then take out the USB stick or disc you started from.")
    backVisible: false
    primaryText: page.backend.rebooting ? qsTr("Restarting…") : qsTr("Restart")
    primaryEnabled: !page.backend.rebooting && (page.recoveryKey.length === 0 || page.keySaved)
    onPrimary: page.backend.reboot()

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff
        QQC2.ScrollBar.vertical: TelamonScrollBar {}

        ColumnLayout {
            width: page.contentWidth
            // Centred on the page, not beside the scroll bar.
            x: Math.round((scroll.width - width) / 2)
            spacing: Kirigami.Units.gridUnit

            QQC2.Label {
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                wrapMode: Text.Wrap
                text: (page.result.windowsEntry === true
                       ? qsTr("Windows is still there: choose it in the menu when the computer starts.") + " "
                       : "")
                      + (page.mok.length > 0
                         ? qsTr("Once Telamon OS starts, you'll create your account.")
                         : qsTr("After the restart, you'll create your account."))
            }

            Section {
                visible: page.recoveryKey.length > 0
                title: qsTr("Recovery Key")
                footer: qsTr("Telamon OS can't show it again after the restart.")

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.gridUnit
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("Write it down or take a photo with your phone, and keep it away from this PC. You need it if Telamon OS ever asks for it, for example after a firmware or security-chip change, or if the disk moves to another PC.")
                    }
                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        implicitWidth: Math.min(keyColumn.implicitWidth + Kirigami.Units.gridUnit * 2, parent.width)
                        implicitHeight: keyColumn.implicitHeight + Kirigami.Units.largeSpacing * 2
                        radius: TelamonStyle.radiusLarge
                        color: TelamonStyle.selection
                        ColumnLayout {
                            id: keyColumn
                            anchors.centerIn: parent
                            spacing: Kirigami.Units.smallSpacing
                            Repeater {
                                model: page.result.recoveryRows || []
                                TextEdit {
                                    required property string modelData
                                    Layout.alignment: Qt.AlignHCenter
                                    text: modelData
                                    readOnly: true
                                    selectByMouse: true
                                    color: Kirigami.Theme.textColor
                                    selectionColor: TelamonStyle.accent
                                    selectedTextColor: TelamonStyle.accentText
                                    font.family: TelamonStyle.monoFamily
                                    font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.25
                                    font.weight: Font.DemiBold
                                    Accessible.role: Accessible.StaticText
                                    Accessible.name: qsTr("Recovery key, part")
                                    Accessible.description: modelData.split("-").join(", ")
                                }
                            }
                        }
                    }
                    // Telamon.Ui's switch, as on the Disk page: Breeze's
                    // check box is a near-invisible square in AtlasOS Dark,
                    // and this one gates Restart.
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.largeSpacing
                        QQC2.Label {
                            Layout.fillWidth: true
                            wrapMode: Text.Wrap
                            text: savedSwitch.Accessible.name
                            TapHandler {
                                onTapped: savedSwitch.toggle()
                            }
                        }
                        TelamonSwitch {
                            id: savedSwitch
                            onCheckedChanged: page.keySaved = checked
                            Accessible.name: qsTr("I've saved my recovery key")
                        }
                    }
                }
            }

            Section {
                visible: page.mok.length > 0
                title: qsTr("One More Step for NVIDIA Graphics")
                footer: qsTr("This happens once. If you miss the blue screen, Telamon OS starts with basic graphics; open a terminal and run “sudo /usr/libexec/atlasos/nvidia-enroll-key” to try again.")

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.gridUnit
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("Secure Boot is on, so the computer must trust the Telamon OS key before the NVIDIA driver can load. After the restart a blue screen appears. It waits only 10 seconds, so stay close:")
                    }
                    Repeater {
                        model: [
                            qsTr("Press any key when it says “Press any key to perform MOK management”."),
                            qsTr("Choose “Enroll MOK”, then “Continue”, then “Yes”."),
                            qsTr("Type the password below with the number keys. Nothing shows while you type."),
                            qsTr("Choose “Reboot”.")
                        ]
                        RowLayout {
                            id: mokStep
                            required property string modelData
                            required property int index
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.largeSpacing
                            QQC2.Label {
                                Layout.alignment: Qt.AlignTop
                                text: (mokStep.index + 1) + "."
                                font.weight: Font.DemiBold
                            }
                            QQC2.Label {
                                Layout.fillWidth: true
                                wrapMode: Text.Wrap
                                text: mokStep.modelData
                            }
                        }
                    }
                    Rectangle {
                        Layout.alignment: Qt.AlignHCenter
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        implicitWidth: pw.implicitWidth + Kirigami.Units.gridUnit * 2
                        implicitHeight: pw.implicitHeight + Kirigami.Units.largeSpacing * 2
                        radius: TelamonStyle.radiusLarge
                        color: TelamonStyle.selection
                        QQC2.Label {
                            id: pw
                            anchors.centerIn: parent
                            text: page.mok.replace(/(\d{4})(\d{4})/, "$1 $2")
                            font.family: TelamonStyle.monoFamily
                            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 2
                            font.weight: Font.DemiBold
                            font.letterSpacing: 2
                            Accessible.name: qsTr("Password: %1").arg(page.mok.split("").join(" "))
                        }
                    }
                }
            }

            Section {
                visible: (page.result.warnings || []).length > 0
                title: qsTr("Worth Knowing")
                footer: qsTr("Telamon OS works despite these. The install log is at %1 until the computer restarts.").arg(page.result.log || "/run/telamon-installer/install.log")
                Repeater {
                    model: page.result.warnings || []
                    SectionRow {
                        required property string modelData
                        iconName: "dialog-warning-symbolic"
                        title: modelData
                    }
                }
            }

            InfoBanner {
                Layout.fillWidth: true
                shown: page.backend.rebootError.length > 0
                type: "error"
                text: page.backend.rebootError
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
