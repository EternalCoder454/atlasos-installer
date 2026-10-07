import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import Telamon.Ui

// A big selectable card for the Disk page's two ways to install. A card
// that isn't possible is greyed out and says why.
FocusScope {
    id: card

    property string title
    property string text
    property string iconName
    property bool selected: false
    property bool possible: true
    property bool destructive: false

    signal clicked

    Layout.fillWidth: true
    implicitHeight: col.implicitHeight + Kirigami.Units.gridUnit * 1.4
    // Kept while focused: Qt refuses to drop it then (a click elsewhere can
    // make the focused card impossible); it goes once the focus moves on.
    activeFocusOnTab: card.possible || card.activeFocus
    Accessible.role: Accessible.RadioButton
    Accessible.name: card.title
    Accessible.description: card.text
    Accessible.checkable: true
    Accessible.checked: card.selected
    Accessible.onPressAction: if (card.possible) card.clicked()
    Keys.onReturnPressed: if (card.possible) card.clicked()
    Keys.onEnterPressed: if (card.possible) card.clicked()
    Keys.onSpacePressed: if (card.possible) card.clicked()

    Rectangle {
        anchors.fill: parent
        radius: TelamonStyle.radiusLarge
        // The Section card's colours, so a card reads as part of the same page.
        color: card.selected ? TelamonStyle.selection : hover.hovered && card.possible ? TelamonStyle.surfaceRaised : TelamonStyle.surface
        border.width: card.selected ? 2 : 1
        border.color: card.selected ? TelamonStyle.accent : hover.hovered && card.possible ? TelamonStyle.controlBorder : TelamonStyle.separator
        Behavior on color {
            ColorAnimation {
                duration: TelamonStyle.durationShort
            }
        }
        TelamonFocusRing {
            shown: card.activeFocus && !tap.pressed
            radius: parent.radius + gap
        }
    }
    HoverHandler {
        id: hover
        cursorShape: card.possible ? Qt.PointingHandCursor : Qt.ArrowCursor
    }
    TapHandler {
        id: tap
        enabled: card.possible
        onTapped: card.clicked()
    }

    RowLayout {
        id: col
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.margins: Kirigami.Units.gridUnit * 0.9
        spacing: Kirigami.Units.gridUnit * 0.8
        opacity: card.possible ? 1 : 0.55

        Rectangle {
            Layout.alignment: Qt.AlignTop
            readonly property real size: Math.round(Kirigami.Units.gridUnit * 2.2)
            Layout.preferredWidth: size
            Layout.preferredHeight: size
            radius: size / 2
            color: Qt.alpha(card.destructive && card.possible ? Kirigami.Theme.negativeTextColor : TelamonStyle.accent, 0.14)
            Kirigami.Icon {
                anchors.centerIn: parent
                width: Kirigami.Units.iconSizes.smallMedium
                height: width
                source: card.iconName
                isMask: true
                color: card.destructive && card.possible ? Kirigami.Theme.negativeTextColor : TelamonStyle.accent
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            QQC2.Label {
                Layout.fillWidth: true
                text: card.title
                font.weight: Font.DemiBold
                font.pointSize: Kirigami.Theme.defaultFont.pointSize * 1.1
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: card.text
                wrapMode: Text.Wrap
                opacity: 0.75
                textFormat: Text.PlainText
            }
        }
        // A radio mark, so the two cards read as one choice.
        Rectangle {
            Layout.alignment: Qt.AlignVCenter
            visible: card.possible
            readonly property real size: Math.round(Kirigami.Units.gridUnit * 1.1)
            Layout.preferredWidth: size
            Layout.preferredHeight: size
            radius: size / 2
            // Filled accent with a light dot when chosen, like a macOS radio.
            color: card.selected ? TelamonStyle.accent : "transparent"
            border.width: card.selected ? 0 : 1
            border.color: Qt.alpha(Kirigami.Theme.textColor, 0.4)
            Rectangle {
                anchors.centerIn: parent
                visible: card.selected
                width: Math.round(parent.size * 0.4)
                height: width
                radius: width / 2
                color: TelamonStyle.accentText
            }
        }
    }
}
