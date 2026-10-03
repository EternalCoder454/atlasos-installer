pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 7: done. Restart, plus the NVIDIA key steps when the helper queued
// the key, and anything that went wrong without failing the install.
InstallerPage {
    id: page

    required property var app

    readonly property var backend: page.app.backend
    readonly property var result: JSON.parse(page.backend.resultJson)
    readonly property string mok: page.result.mokPassword || ""

    title: qsTr("AtlasOS is installed")
    subtitle: qsTr("Remove the USB stick, then restart.")
    backVisible: false
    primaryText: page.backend.rebooting ? qsTr("Restarting…") : qsTr("Restart")
    primaryEnabled: !page.backend.rebooting
    onPrimary: page.backend.reboot()

    QQC2.ScrollView {
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

        ColumnLayout {
            width: page.contentWidth
            // Centred on the page, not beside the scroll bar.
            x: Math.round((scroll.width - width) / 2)
            spacing: Kirigami.Units.gridUnit

            QQC2.Label {
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                text: (page.result.windowsEntry === true
                       ? qsTr("Windows is still there: choose it in the menu when the computer starts.") + " "
                       : "")
                      + (page.mok.length > 0
                         ? qsTr("Once AtlasOS starts, you'll create your account.")
                         : qsTr("After the restart, you'll create your account."))
            }

            Section {
                visible: page.mok.length > 0
                title: qsTr("One more step for NVIDIA graphics")
                footer: qsTr("This happens once. If you miss the blue screen, AtlasOS starts with basic graphics; open a terminal and run “sudo /usr/libexec/atlasos/nvidia-enroll-key” to try again.")

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.margins: Kirigami.Units.gridUnit
                    spacing: Kirigami.Units.largeSpacing

                    QQC2.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: qsTr("Secure Boot is on, so the computer must trust the AtlasOS key before the NVIDIA driver can load. After the restart a blue screen appears. It waits only 10 seconds, so stay close:")
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
                        radius: 8
                        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.12)
                        QQC2.Label {
                            id: pw
                            anchors.centerIn: parent
                            text: page.mok.replace(/(\d{4})(\d{4})/, "$1 $2")
                            font.family: "monospace"
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
                title: qsTr("Worth knowing")
                footer: qsTr("AtlasOS works despite these. The install log is at %1 until the computer restarts.").arg(page.result.log || "/run/atlas-installer/install.log")
                Repeater {
                    model: page.result.warnings || []
                    SectionRow {
                        required property string modelData
                        iconName: "dialog-warning-symbolic"
                        title: modelData
                    }
                }
            }

            Note {
                Layout.fillWidth: true
                visible: page.backend.rebootError.length > 0
                kind: "error"
                text: page.backend.rebootError
            }

            Item {
                implicitHeight: Kirigami.Units.gridUnit
            }
        }
    }
}
