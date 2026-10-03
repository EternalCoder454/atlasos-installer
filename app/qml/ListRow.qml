import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

// A row in a ListFrame: icon, title and subtitle, trailing items, and a
// checkmark when selected. Draws its separator from its index, which works
// with a ListView (SectionRow's own separator logic does not).
FocusScope {
    id: row

    required property int index
    property string title
    property string subtitle
    property string iconName
    property bool selected: false
    property bool clickable: true
    default property alias trailing: trailingRow.data
    // Shown under the title row when the row is expanded (e.g. a password field).
    property alias expansion: expansionCol.data
    property bool expanded: false

    signal clicked

    width: ListView.view ? ListView.view.width : implicitWidth
    implicitHeight: Math.max(Math.round(Kirigami.Units.gridUnit * 2.5), top.implicitHeight + Kirigami.Units.largeSpacing * 1.6) + (row.expanded ? expansionCol.implicitHeight + Kirigami.Units.largeSpacing : 0)
    activeFocusOnTab: false
    Accessible.role: Accessible.ListItem
    Accessible.name: row.title
    Accessible.description: row.subtitle
    Accessible.selectable: true
    Accessible.selected: row.selected
    Accessible.onPressAction: if (row.clickable) row.clicked()

    Keys.onReturnPressed: event => row.activate(event)
    Keys.onEnterPressed: event => row.activate(event)
    Keys.onSpacePressed: event => row.activate(event)

    function activate(event) {
        if (row.clickable && !event.isAutoRepeat) {
            row.clicked();
        }
        event.accepted = row.clickable;
    }

    Rectangle {
        visible: row.index > 0
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.leftMargin: Kirigami.Units.largeSpacing + (row.iconName.length > 0 ? Kirigami.Units.iconSizes.smallMedium + Kirigami.Units.largeSpacing : 0)
        height: 1
        color: Qt.alpha(Kirigami.Theme.textColor, 0.1)
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: 3
        radius: 7
        color: row.selected ? Qt.alpha(Kirigami.Theme.highlightColor, 0.14) : Qt.alpha(Kirigami.Theme.textColor, tap.pressed ? 0.1 : 0.05)
        opacity: row.selected || (row.clickable && hover.hovered) ? 1 : 0
    }
    Rectangle {
        anchors.fill: parent
        anchors.margins: 3
        radius: 7
        color: "transparent"
        border.width: 2
        border.color: Qt.alpha(Kirigami.Theme.highlightColor, 0.6)
        visible: row.ListView.isCurrentItem && row.ListView.view.activeFocus
    }

    HoverHandler {
        id: hover
        enabled: row.clickable
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        id: tap
        enabled: row.clickable
        // Taps in the expansion (password field, buttons) are theirs.
        onTapped: (point) => {
            if (point.position.y <= top.y + top.height + Kirigami.Units.largeSpacing) {
                if (row.ListView.view) {
                    row.ListView.view.currentIndex = row.index;
                }
                row.clicked();
            }
        }
    }

    RowLayout {
        id: top
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.topMargin: Math.round((Math.max(Math.round(Kirigami.Units.gridUnit * 2.5), implicitHeight + Kirigami.Units.largeSpacing * 1.6) - implicitHeight) / 2)
        anchors.leftMargin: Kirigami.Units.largeSpacing
        anchors.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing
        opacity: row.clickable || row.selected ? 1 : 0.5

        Kirigami.Icon {
            visible: row.iconName.length > 0
            source: row.iconName
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            QQC2.Label {
                Layout.fillWidth: true
                text: row.title
                elide: Text.ElideRight
                textFormat: Text.PlainText
                font.weight: row.selected ? Font.DemiBold : Font.Normal
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: row.subtitle.length > 0
                text: row.subtitle
                wrapMode: Text.Wrap
                font: Kirigami.Theme.smallFont
                opacity: 0.65
                textFormat: Text.PlainText
            }
        }
        Row {
            id: trailingRow
            spacing: Kirigami.Units.smallSpacing
        }
        Kirigami.Icon {
            visible: row.selected
            source: "checkmark"
            isMask: true
            color: Kirigami.Theme.highlightColor
            Layout.preferredWidth: Kirigami.Units.iconSizes.smallMedium
            Layout.preferredHeight: Kirigami.Units.iconSizes.smallMedium
        }
    }

    ColumnLayout {
        id: expansionCol
        visible: row.expanded
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.leftMargin: Kirigami.Units.largeSpacing + (row.iconName.length > 0 ? Kirigami.Units.iconSizes.smallMedium + Kirigami.Units.largeSpacing : 0)
        anchors.rightMargin: Kirigami.Units.largeSpacing
        anchors.bottomMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing
    }
}
