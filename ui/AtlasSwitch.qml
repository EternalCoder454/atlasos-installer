import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// Pill switch: accent track when on, white knob.
T.Switch {
    id: control

    implicitWidth: 40
    implicitHeight: 24
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    indicator: Rectangle {
        implicitWidth: 40
        implicitHeight: 24
        x: control.leftPadding
        y: Math.round((control.height - height) / 2)
        radius: height / 2
        color: control.checked ? Kirigami.Theme.highlightColor : Qt.alpha(Kirigami.Theme.textColor, control.hovered ? 0.28 : 0.2)
        border.width: control.visualFocus ? 2 : 0
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        opacity: control.enabled ? 1 : 0.45
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
        Rectangle {
            width: 20
            height: 20
            radius: 10
            y: 2
            x: (control.checked !== control.mirrored) ? parent.width - width - 2 : 2
            color: "white"
            Behavior on x {
                NumberAnimation {
                    duration: Kirigami.Units.shortDuration
                    easing.type: Easing.OutCubic
                }
            }
        }
    }
    contentItem: Item {}
}
