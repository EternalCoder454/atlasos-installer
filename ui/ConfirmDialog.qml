import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// Modal dialog in the Atlas look: rounded card, pill buttons.
QQC2.Popup {
    id: dialog

    property string title
    property string text
    property string acceptText: qsTr("OK")
    property string rejectText: qsTr("Cancel")
    property bool showReject: true
    // Destructive dialogs start on Cancel; informational ones on the main button.
    property bool focusReject: false
    property bool closeOnAccept: true
    default property alias body: bodyColumn.data

    signal accepted

    parent: QQC2.Overlay.overlay
    anchors.centerIn: parent
    modal: true
    focus: true
    closePolicy: QQC2.Popup.CloseOnEscape | QQC2.Popup.CloseOnPressOutside
    width: Math.min(parent ? parent.width - Kirigami.Units.gridUnit * 2 : 0, Kirigami.Units.gridUnit * 25)
    padding: Math.round(Kirigami.Units.gridUnit * 1.3)
    height: Math.min(implicitHeight, parent ? parent.height - Kirigami.Units.gridUnit * 2 : implicitHeight)
    onOpened: (dialog.focusReject && dialog.showReject ? rejectButton : acceptButton).forceActiveFocus()

    enter: Transition {
        NumberAnimation {
            property: "opacity"
            from: 0
            to: 1
            duration: Kirigami.Units.shortDuration
        }
    }
    exit: Transition {
        NumberAnimation {
            property: "opacity"
            from: 1
            to: 0
            duration: Kirigami.Units.shortDuration
        }
    }

    QQC2.Overlay.modal: Rectangle {
        color: Qt.rgba(0, 0, 0, 0.35)
    }

    background: Rectangle {
        radius: 14
        color: Kirigami.Theme.backgroundColor
        border.width: 1
        border.color: Qt.alpha(Kirigami.Theme.textColor, 0.16)
    }

    contentItem: ColumnLayout {
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.title
        spacing: Kirigami.Units.largeSpacing
        QQC2.Label {
            Layout.fillWidth: true
            text: dialog.title
            font.bold: true
            font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.15
            wrapMode: Text.Wrap
            textFormat: Text.PlainText
            Accessible.role: Accessible.Heading
        }
        QQC2.Label {
            Layout.fillWidth: true
            visible: dialog.text.length > 0
            text: dialog.text
            wrapMode: Text.Wrap
            opacity: 0.8
            textFormat: Text.PlainText
        }
        ColumnLayout {
            id: bodyColumn
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Kirigami.Units.smallSpacing
            spacing: Kirigami.Units.largeSpacing
            Item {
                Layout.fillWidth: true
            }
            SecondaryButton {
                id: rejectButton
                visible: dialog.showReject
                text: dialog.rejectText
                onClicked: dialog.close()
            }
            PrimaryButton {
                id: acceptButton
                text: dialog.acceptText
                onClicked: {
                    dialog.accepted();
                    if (dialog.closeOnAccept) {
                        dialog.close();
                    }
                }
            }
        }
    }
}
