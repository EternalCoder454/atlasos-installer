import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// One step in a setup sidebar: a numbered circle (a checkmark once done)
// and the step's name. The current step gets the selection pill. Only done
// steps can be clicked, to go back to them.
T.AbstractButton {
    id: control

    property int number: 1
    property bool current: false
    property bool done: false
    // Not `enabled`: a disabled item gets Kirigami's disabled colours, and
    // the current step must keep its accent.
    property bool clickable: control.done && !control.current

    implicitHeight: Math.round(Kirigami.Units.gridUnit * 2.1)
    implicitWidth: Kirigami.Units.gridUnit * 10
    hoverEnabled: control.clickable
    focusPolicy: control.clickable ? Qt.StrongFocus : Qt.NoFocus
    Accessible.role: Accessible.ListItem
    Accessible.name: control.text
    Accessible.description: control.current ? qsTr("Current step") : control.done ? qsTr("Done") : qsTr("Not done yet")
    Keys.onReturnPressed: event => {
        if (control.clickable && !event.isAutoRepeat) {
            control.clicked();
        }
    }
    Keys.onEnterPressed: event => {
        if (control.clickable && !event.isAutoRepeat) {
            control.clicked();
        }
    }

    // Swallow presses on steps that can't be opened.
    MouseArea {
        anchors.fill: parent
        enabled: !control.clickable
        acceptedButtons: Qt.AllButtons
    }

    background: Rectangle {
        radius: 8
        color: control.current ? Qt.alpha(Kirigami.Theme.highlightColor, 0.18) : Qt.alpha(Kirigami.Theme.textColor, control.down ? 0.1 : control.hovered ? 0.06 : 0)
        border.width: control.visualFocus ? 2 : 0
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
    }

    contentItem: RowLayout {
        spacing: Kirigami.Units.largeSpacing
        Rectangle {
            id: dot
            Layout.leftMargin: Kirigami.Units.largeSpacing
            readonly property real size: Math.round(Kirigami.Units.gridUnit * 1.2)
            Layout.preferredWidth: size
            Layout.preferredHeight: size
            radius: size / 2
            color: control.current || control.done ? Kirigami.Theme.highlightColor : "transparent"
            border.width: control.current || control.done ? 0 : 1
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.35)
            Text {
                anchors.centerIn: parent
                visible: !control.done || control.current
                text: control.number
                font.family: Kirigami.Theme.defaultFont.family
                font.pointSize: Kirigami.Theme.smallFont.pointSize
                font.weight: Font.DemiBold
                color: control.current ? Kirigami.Theme.highlightedTextColor : Qt.alpha(Kirigami.Theme.textColor, 0.6)
            }
            Kirigami.Icon {
                anchors.centerIn: parent
                visible: control.done && !control.current
                width: Math.round(dot.size * 0.7)
                height: width
                source: "checkmark"
                isMask: true
                color: Kirigami.Theme.highlightedTextColor
            }
        }
        Text {
            Layout.fillWidth: true
            text: control.text
            font.family: Kirigami.Theme.defaultFont.family
            font.pointSize: Kirigami.Theme.defaultFont.pointSize
            font.weight: control.current ? Font.DemiBold : Font.Medium
            textFormat: Text.PlainText
            elide: Text.ElideRight
            color: Kirigami.Theme.textColor
            opacity: control.current || control.done ? 1 : 0.6
        }
    }
}
