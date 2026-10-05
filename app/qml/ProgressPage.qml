import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// Step 7: the install. No cancel: once the disk is being changed, stopping
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

    // The time left counts down between the helper's progress updates.
    Timer {
        interval: 1000
        repeat: true
        running: page.visible && page.backend.installState === "running"
        onTriggered: page.backend.tickTimeLeft()
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(page.contentWidth, Kirigami.Units.gridUnit * 26)
        spacing: Kirigami.Units.gridUnit
        visible: !page.failed

        QQC2.Label {
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 2.1
            font.weight: Font.DemiBold
            font.letterSpacing: -0.2
            text: qsTr("Installing AtlasOS")
            textFormat: Text.PlainText
            Accessible.role: Accessible.Heading
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
                // digits of one width: the countdown doesn't wobble
                font.features: { "tnum": 1 }
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
            headline: page.failed ? qsTr("AtlasOS couldn't be installed") : ""
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
        InfoBanner {
            Layout.fillWidth: true
            shown: page.backend.rebootError.length > 0
            type: "error"
            text: page.backend.rebootError
        }
    }
}
