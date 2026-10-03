import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 6: the install. No cancel: once the disk is being changed, stopping
// halfway would leave it worse off. On failure, what happened and what next.
InstallerPage {
    id: page

    required property var app

    readonly property var backend: page.app.backend
    readonly property bool failed: page.backend.installState === "failed"

    // Only after a failure: try another disk, or give up and restart.
    footerVisible: page.failed
    backVisible: false
    primaryText: qsTr("Choose a Disk Again")
    secondaryText: page.backend.rebooting ? qsTr("Restarting…") : qsTr("Restart")
    onPrimary: page.app.chooseDiskAgain()
    onSecondary: page.backend.reboot()

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(page.contentWidth, Kirigami.Units.gridUnit * 26)
        spacing: Kirigami.Units.gridUnit
        visible: !page.failed

        Image {
            Layout.alignment: Qt.AlignHCenter
            source: "qrc:/qt/qml/net/eterneon/atlas/installer/data/atlasos-logo.svg"
            sourceSize.width: Kirigami.Units.gridUnit * 5
            sourceSize.height: Kirigami.Units.gridUnit * 5
            Accessible.ignored: true
        }
        Kirigami.Heading {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            level: 1
            font.weight: Font.DemiBold
            text: qsTr("Installing AtlasOS")
            textFormat: Text.PlainText
        }
        AtlasProgressBar {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.largeSpacing
            value: page.backend.progress
        }
        RowLayout {
            Layout.fillWidth: true
            QQC2.Label {
                Layout.fillWidth: true
                text: page.backend.progressText
                elide: Text.ElideRight
                textFormat: Text.PlainText
            }
            QQC2.Label {
                text: page.backend.timeLeft
                opacity: 0.65
                textFormat: Text.PlainText
            }
        }
        QQC2.Label {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.gridUnit
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            opacity: 0.6
            font: Kirigami.Theme.smallFont
            text: qsTr("Keep the computer on and the installer media connected.")
        }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(page.contentWidth, Kirigami.Units.gridUnit * 30)
        spacing: Kirigami.Units.gridUnit
        visible: page.failed

        StatusHero {
            iconName: "window-close-symbolic"
            tint: Kirigami.Theme.negativeTextColor
            headline: page.failed ? qsTr("AtlasOS Couldn't Be Installed") : ""
            subtitle: page.backend.installError
        }
        QQC2.Label {
            Layout.fillWidth: true
            // No log when the helper never started (polkit said no); the
            // helper's message usually names the log already.
            visible: page.backend.installBegan
                     && page.backend.installError.indexOf("/run/atlas-installer/") < 0
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            opacity: 0.6
            font: Kirigami.Theme.smallFont
            text: qsTr("The install log is at /run/atlas-installer/install.log until the computer restarts.")
        }
        Note {
            Layout.fillWidth: true
            visible: page.backend.rebootError.length > 0
            kind: "error"
            text: page.backend.rebootError
        }
    }
}
