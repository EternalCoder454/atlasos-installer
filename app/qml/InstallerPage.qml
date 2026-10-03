import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One installer step: the question as a big title and a line under it, the
// page's content, and a footer with a quiet Back on the left and the accent
// pill on the right.
FocusScope {
    id: page

    property string title
    property string subtitle
    property string primaryText: qsTr("Continue")
    property bool primaryEnabled: true
    property bool backVisible: true
    property bool footerVisible: true
    // A text button beside the pill ("Set Up Later"), or "".
    property string secondaryText
    property alias header: headerSlot.data
    default property alias content: body.data
    // Lines stay readable: wider windows get more margin, not longer lines.
    readonly property real contentWidth: Math.min(width - Kirigami.Units.gridUnit * 4, Kirigami.Units.gridUnit * 34)

    signal primary
    signal back
    signal secondary

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ColumnLayout {
            id: headerCol
            // A child layout fills both ways unless told otherwise.
            Layout.fillWidth: false
            Layout.fillHeight: false
            Layout.preferredWidth: page.contentWidth
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: Kirigami.Units.gridUnit * 2.5
            spacing: Kirigami.Units.largeSpacing

            ColumnLayout {
                id: headerSlot
                Layout.fillWidth: true
                visible: children.length > 0
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: page.title.length > 0
                text: page.title
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 2.1
                font.weight: Font.DemiBold
                font.letterSpacing: -0.2
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                Accessible.role: Accessible.Heading
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: page.subtitle.length > 0
                text: page.subtitle
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.15
                wrapMode: Text.Wrap
                opacity: 0.7
                textFormat: Text.PlainText
            }
        }

        Item {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: Kirigami.Units.gridUnit * 1.5
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.gridUnit
            Layout.leftMargin: Kirigami.Units.gridUnit * 1.5
            Layout.rightMargin: Kirigami.Units.gridUnit * 1.5
            visible: page.footerVisible
            spacing: Kirigami.Units.largeSpacing

            BackButton {
                visible: page.backVisible
                onClicked: page.back()
            }
            Item {
                Layout.fillWidth: true
            }
            TextButton {
                visible: page.secondaryText.length > 0
                text: page.secondaryText
                onClicked: page.secondary()
            }
            PrimaryButton {
                id: primaryButton
                text: page.primaryText
                enabled: page.primaryEnabled
                onClicked: page.primary()
            }
        }
    }
}
