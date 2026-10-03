import QtQuick
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// Shared pill button. Use PrimaryButton or SecondaryButton.
T.AbstractButton {
    id: control

    property bool prominent: false
    readonly property color accent: Kirigami.Theme.highlightColor
    readonly property color textTint: Kirigami.Theme.textColor

    implicitWidth: Math.max(Math.round(Kirigami.Units.gridUnit * 4.5), contentItem.implicitWidth + leftPadding + rightPadding)
    implicitHeight: Math.round(Kirigami.Units.gridUnit * 1.7)
    leftPadding: Kirigami.Units.largeSpacing + Kirigami.Units.smallSpacing
    rightPadding: leftPadding
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    scale: control.down && control.enabled ? 0.97 : 1

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

    Behavior on scale {
        NumberAnimation {
            duration: Kirigami.Units.shortDuration
            easing.type: Easing.OutCubic
        }
    }

    contentItem: Item {
        implicitWidth: row.implicitWidth
        implicitHeight: row.implicitHeight
        Row {
            id: row
            anchors.centerIn: parent
            spacing: Kirigami.Units.smallSpacing
            Kirigami.Icon {
                visible: control.icon.name.length > 0
                source: control.icon.name
                isMask: true
                color: label.color
                width: Kirigami.Units.iconSizes.small
                height: width
                anchors.verticalCenter: parent.verticalCenter
            }
            Text {
                id: label
                anchors.verticalCenter: parent.verticalCenter
                text: control.text
                font: Kirigami.Theme.defaultFont
                color: control.prominent && control.enabled ? Kirigami.Theme.highlightedTextColor : control.textTint
                opacity: control.enabled ? 1 : 0.75
                textFormat: Text.PlainText // no mnemonics
            }
        }
    }

    background: Rectangle {
        radius: height / 2
        color: {
            if (control.prominent) {
                if (!control.enabled) {
                    return Qt.alpha(control.textTint, 0.12);
                }
                return control.down ? Qt.darker(control.accent, 1.2) : control.hovered ? Qt.lighter(control.accent, 1.12) : control.accent;
            }
            return Qt.alpha(control.textTint, control.down ? 0.2 : control.hovered ? 0.12 : 0.07);
        }
        border.width: control.prominent ? 0 : 1
        border.color: Qt.alpha(control.textTint, 0.14)
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: -3
            radius: height / 2
            color: "transparent"
            border.width: 2
            border.color: Qt.alpha(control.accent, 0.6)
            visible: control.visualFocus
        }
    }
}
