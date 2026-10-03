import QtQuick
import org.kde.kirigami as Kirigami

// A rounded progress bar in the accent colour. `value` runs from 0 to 1;
// `indeterminate` slides a short segment back and forth instead.
Item {
    id: root

    property real value: 0
    property bool indeterminate: false

    implicitWidth: Kirigami.Units.gridUnit * 16
    implicitHeight: Math.round(Kirigami.Units.gridUnit * 0.45)

    Accessible.role: Accessible.ProgressBar
    Accessible.name: root.indeterminate ? "" : Math.round(root.value * 100) + "%"

    Rectangle {
        id: track
        anchors.fill: parent
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.textColor, 0.12)
        clip: true

        Rectangle {
            visible: !root.indeterminate
            height: parent.height
            radius: height / 2
            width: root.value > 0 ? Math.max(height, parent.width * Math.min(1, root.value)) : 0
            color: Kirigami.Theme.highlightColor
            Behavior on width {
                NumberAnimation {
                    duration: Kirigami.Units.longDuration
                    easing.type: Easing.OutCubic
                }
            }
        }

        Rectangle {
            id: slider
            visible: root.indeterminate
            height: parent.height
            radius: height / 2
            width: parent.width * 0.3
            color: Kirigami.Theme.highlightColor
            SequentialAnimation on x {
                running: root.indeterminate && root.visible && Kirigami.Units.longDuration > 0
                loops: Animation.Infinite
                NumberAnimation {
                    from: 0
                    to: track.width - slider.width
                    duration: Kirigami.Units.veryLongDuration * 2
                    easing.type: Easing.InOutQuad
                }
                NumberAnimation {
                    from: track.width - slider.width
                    to: 0
                    duration: Kirigami.Units.veryLongDuration * 2
                    easing.type: Easing.InOutQuad
                }
            }
        }
    }
}
