import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// Link-styled button: accent text, no fill.
T.AbstractButton {
    id: control

    implicitWidth: label.implicitWidth + leftPadding + rightPadding
    implicitHeight: Math.round(Kirigami.Units.gridUnit * 1.7)
    leftPadding: Kirigami.Units.smallSpacing
    rightPadding: leftPadding
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    Accessible.name: control.text
    Keys.onReturnPressed: event => {
        if (enabled && !event.isAutoRepeat) {
            control.clicked();
        }
    }
    Keys.onEnterPressed: event => {
        if (enabled && !event.isAutoRepeat) {
            control.clicked();
        }
    }

    contentItem: Text {
        id: label
        verticalAlignment: Text.AlignVCenter
        text: control.text
        font: Kirigami.Theme.defaultFont
        textFormat: Text.PlainText
        color: control.down ? Qt.darker(Kirigami.Theme.highlightColor, 1.2) : control.hovered ? Qt.lighter(Kirigami.Theme.highlightColor, 1.15) : Kirigami.Theme.highlightColor
        opacity: control.enabled ? 1 : 0.45
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
    }
    background: Rectangle {
        radius: 6
        color: "transparent"
        border.width: control.visualFocus ? 2 : 0
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
    }
}
