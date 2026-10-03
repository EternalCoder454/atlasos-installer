import QtQuick
import QtQuick.Layouts
import QtQuick.Templates as T
import org.kde.kirigami as Kirigami

// A quiet Back: text and an arrow, no fill, so the page's one filled
// button is the way forward.
T.AbstractButton {
    id: control

    text: qsTr("Back")
    implicitWidth: row.implicitWidth + leftPadding + rightPadding
    implicitHeight: Math.round(Kirigami.Units.gridUnit * 2)
    leftPadding: Kirigami.Units.largeSpacing
    rightPadding: Kirigami.Units.largeSpacing * 1.5
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

    contentItem: RowLayout {
        id: row
        spacing: Kirigami.Units.smallSpacing
        Kirigami.Icon {
            Layout.preferredWidth: Kirigami.Units.iconSizes.small
            Layout.preferredHeight: Kirigami.Units.iconSizes.small
            source: LayoutMirroring.enabled ? "go-next-symbolic" : "go-previous-symbolic"
            isMask: true
            color: Kirigami.Theme.textColor
            opacity: 0.75
        }
        Text {
            text: control.text
            font: Kirigami.Theme.defaultFont
            color: Kirigami.Theme.textColor
            opacity: 0.85
            textFormat: Text.PlainText
        }
    }

    background: Rectangle {
        radius: height / 2
        color: Qt.alpha(Kirigami.Theme.textColor, control.down ? 0.12 : control.hovered ? 0.07 : 0)
        border.width: control.visualFocus ? 2 : 0
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        Behavior on color {
            ColorAnimation {
                duration: Kirigami.Units.shortDuration
            }
        }
    }
}
