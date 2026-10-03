import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Atlas.Ui

// One installer step: a big title and a line under it, the page's content,
// and a footer with a quiet Back on the left and the accent pill on the right.
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
    readonly property real contentWidth: Math.min(width - Kirigami.Units.gridUnit * 4, Kirigami.Units.gridUnit * 38)

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
            Layout.topMargin: Kirigami.Units.gridUnit * 2
            spacing: Kirigami.Units.smallSpacing

            ColumnLayout {
                id: headerSlot
                Layout.fillWidth: true
                visible: children.length > 0
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: page.title.length > 0
                text: page.title
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.8
                font.weight: Font.Bold
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                Accessible.role: Accessible.Heading
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: page.subtitle.length > 0
                text: page.subtitle
                wrapMode: Text.Wrap
                opacity: 0.7
                textFormat: Text.PlainText
            }
        }

        Item {
            id: body
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: Kirigami.Units.gridUnit
        }

        Rectangle {
            Layout.fillWidth: true
            visible: page.footerVisible
            implicitHeight: 1
            color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.gridUnit
            Layout.leftMargin: Kirigami.Units.gridUnit * 1.5
            Layout.rightMargin: Kirigami.Units.gridUnit * 1.5
            visible: page.footerVisible
            spacing: Kirigami.Units.largeSpacing

            SecondaryButton {
                visible: page.backVisible
                text: qsTr("Back")
                icon.name: LayoutMirroring.enabled ? "go-next" : "go-previous"
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
